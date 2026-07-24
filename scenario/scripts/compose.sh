#!/usr/bin/env bash
# Docker orchestration for compose scenarios (1c / 2c / 3c topologies).
#
# Set COMPOSE_SPEC=compose-3c|compose-2c|compose-1c (default compose-3c).
# Business logic lives in the Rust `scenario` binary.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE_SPEC="${COMPOSE_SPEC:-compose-3c}"
ARTIFACTS="${REPO_ROOT}/scenario/output/${COMPOSE_SPEC}"
COMPOSE_FILE="${REPO_ROOT}/scenario/compose/${COMPOSE_SPEC}.yml"
PROJECT_NAME="sinkd_${COMPOSE_SPEC//-/_}"
COMPOSE=(docker compose -f "${COMPOSE_FILE}" -p "${PROJECT_NAME}")

if [[ -n "${SCENARIO_BIN:-}" ]]; then
    SCENARIO=("${SCENARIO_BIN}")
else
    SCENARIO=(cargo run --quiet -p scenario --manifest-path "${REPO_ROOT}/scenario/Cargo.toml" --)
fi

scenario() {
    "${SCENARIO[@]}" compose "$@"
}

log() { printf '[compose:%s] %s\n' "${COMPOSE_SPEC}" "$*"; }

ensure_ssh_keys() {
    local ssh_dir="${ARTIFACTS}/ssh"
    mkdir -p "${ssh_dir}"
    if [[ ! -f "${ssh_dir}/id_ed25519" ]]; then
        log "generating scenario SSH keypair"
        ssh-keygen -t ed25519 -N "" -f "${ssh_dir}/id_ed25519"
        cp "${ssh_dir}/id_ed25519.pub" "${ssh_dir}/authorized_keys"
    fi
    export SINKD_SSH_DIR="${ssh_dir}"
}

topology_config() {
    case "${COMPOSE_SPEC}" in
        compose-1c)
            SERVER_SVC=sinkd
            LOG_SERVICES=(sinkd)
            CLIENT_USERS=(alice)
            alice_events_svc=sinkd
            ;;
        compose-2c)
            SERVER_SVC=server-alice
            LOG_SERVICES=(server-alice client-bob)
            CLIENT_USERS=(alice bob)
            alice_events_svc=server-alice
            ;;
        compose-3c)
            SERVER_SVC=server
            LOG_SERVICES=(server client-alice client-bob)
            CLIENT_USERS=(alice bob)
            alice_events_svc=client-alice
            ;;
        *)
            echo "compose.sh: unknown COMPOSE_SPEC='${COMPOSE_SPEC}'" >&2
            exit 2
            ;;
    esac
}

cleanup() {
    local rc=$?
    "${COMPOSE[@]}" logs --no-color > "${ARTIFACTS}/compose.log" 2>&1 || true
    for svc in "${LOG_SERVICES[@]}"; do
        "${COMPOSE[@]}" exec -T "${svc}" \
            bash -c 'for f in /var/log/sinkd/*.log; do echo ===== "$f" =====; cat "$f"; done' \
            > "${ARTIFACTS}/${svc}.sinkd.log" 2>&1 || true
    done
    "${COMPOSE[@]}" exec -T "${SERVER_SVC}" \
        cat /srv/sinkd/generation_state.toml 2>/dev/null \
        > "${ARTIFACTS}/server.generation_state.toml" || true
    if [[ "${COMPOSE_KEEP:-0}" == "1" && $rc -ne 0 ]]; then
        log "COMPOSE_KEEP=1 and rc=$rc; leaving stack running for inspection"
        return
    fi
    log "tearing down compose stack"
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
}

trap cleanup EXIT

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "compose.sh: required tool not found: $1" >&2
        exit 127
    fi
}
require docker
require ssh-keygen
if ! docker compose version >/dev/null 2>&1; then
    echo "compose.sh: 'docker compose' plugin is required" >&2
    exit 127
fi

topology_config
mkdir -p "${ARTIFACTS}"
ensure_ssh_keys

fetch_generation() {
    "${COMPOSE[@]}" exec -T "${SERVER_SVC}" \
        cat /srv/sinkd/generation_state.toml 2>/dev/null \
        > "${ARTIFACTS}/generation_state.toml" || true
}

wait_for_path() {
    local svc="$1" path="$2" timeout="${3:-180}"
    local elapsed=0
    while (( elapsed < timeout )); do
        if "${COMPOSE[@]}" exec -T "${svc}" test -f "${path}" >/dev/null 2>&1; then
            return 0
        fi
        sleep 2
        elapsed=$(( elapsed + 2 ))
    done
    echo "compose.sh: timed out waiting ${timeout}s for ${svc}:${path}" >&2
    "${COMPOSE[@]}" logs --tail 200 "${svc}" >&2 || true
    return 1
}

wait_for_generation() {
    local minimum="$1" timeout="${2:-120}"
    local elapsed=0
    while (( elapsed < timeout )); do
        fetch_generation
        if scenario check-generation --minimum "${minimum}" --file "${ARTIFACTS}/generation_state.toml" 2>/dev/null; then
            return 0
        fi
        sleep 2
        elapsed=$(( elapsed + 2 ))
    done
    echo "compose.sh: server generation never reached ${minimum}" >&2
    "${COMPOSE[@]}" logs --tail 200 "${SERVER_SVC}" >&2 || true
    return 1
}

wait_for_generation_stable() {
    local settled=0 prev=""
    for _ in $(seq 1 45); do
        fetch_generation
        cur="$(scenario get-generation --file "${ARTIFACTS}/generation_state.toml" 2>/dev/null || true)"
        if [[ -n "${cur}" && "${cur}" == "${prev}" ]]; then
            settled=$(( settled + 1 ))
            if (( settled >= 4 )); then
                log "server generation stable at ${cur}"
                break
            fi
        else
            settled=0
            prev="${cur}"
        fi
        sleep 2
    done
    sleep 6
}

wait_for_server_mirror_dir() {
    local path="$1" timeout="${2:-180}"
    local elapsed=0
    while (( elapsed < timeout )); do
        if "${COMPOSE[@]}" exec -T "${SERVER_SVC}" test -d "${path}" >/dev/null 2>&1; then
            log "server mirror ready: ${path}"
            return 0
        fi
        sleep 2
        elapsed=$(( elapsed + 2 ))
    done
    echo "compose.sh: timed out waiting for server dir ${path}" >&2
    "${COMPOSE[@]}" logs --tail 120 "${SERVER_SVC}" >&2 || true
    return 1
}

client_service() {
    local user="$1"
    case "${COMPOSE_SPEC}" in
        compose-1c|compose-2c)
            if [[ "${user}" == alice ]]; then
                echo "${SERVER_SVC}"
            else
                echo "client-${user}"
            fi
            ;;
        compose-3c) echo "client-${user}" ;;
    esac
}

copy_tree_from_container() {
    local svc="$1" remote_dir="$2" dest="$3"
    rm -rf "${dest}"
    mkdir -p "${dest}"
    "${COMPOSE[@]}" cp "${svc}:${remote_dir}/." "${dest}/"
}

snapshot_diff() {
    local user="$1" watch="$2" label="$3"
    local svc
    svc="$(client_service "${user}")"
    local client_manifest="${ARTIFACTS}/${label}-${user}.client.manifest"
    local server_manifest="${ARTIFACTS}/${label}-${user}.server.manifest"
    local tmp_client="${ARTIFACTS}/tmp-${label}-${user}-client"
    local tmp_server="${ARTIFACTS}/tmp-${label}-${user}-server"

    copy_tree_from_container "${svc}" "${watch}" "${tmp_client}"
    copy_tree_from_container "${SERVER_SVC}" "/srv/sinkd${watch}" "${tmp_server}"

    scenario snapshot-dir "${tmp_client}" > "${client_manifest}"
    scenario snapshot-dir "${tmp_server}" > "${server_manifest}"
    scenario compare-manifests \
        --client "${client_manifest}" \
        --server "${server_manifest}" \
        --label "${label}" \
        --user "${user}"

    rm -rf "${tmp_client}" "${tmp_server}"
}

phase_b_service() {
    local nominal="$1"
    case "${nominal}" in
        client-alice) client_service alice ;;
        client-bob) client_service bob ;;
        *) echo "${nominal}" ;;
    esac
}

run_phase_b() {
    while IFS=$'\t' read -r nominal op arg1 arg2; do
        case "${user_filter:-}" in
            bob) [[ "${nominal}" != client-bob ]] && continue ;;
            alice) [[ "${nominal}" != client-alice ]] && continue ;;
        esac
        local svc
        svc="$(phase_b_service "${nominal}")"
        case "${op}" in
            rm)
                "${COMPOSE[@]}" exec -T "${svc}" rm -f "${arg1}"
                ;;
            write)
                "${COMPOSE[@]}" exec -T "${svc}" bash -c "echo \"${arg2}\" > \"${arg1}\""
                ;;
            mv)
                "${COMPOSE[@]}" exec -T "${svc}" mv "${arg1}" "${arg2}"
                ;;
            *)
                echo "compose.sh: unknown phase-B op: ${op}" >&2
                return 1
                ;;
        esac
        sleep 3
    done < <(scenario phase-b-ops)
}

bring_up() {
    case "${COMPOSE_SPEC}" in
        compose-1c)
            log "starting sinkd (server + alice)"
            "${COMPOSE[@]}" up -d sinkd
            log "waiting for alice events_done"
            wait_for_path sinkd /watch_alice/.events_done 300
            ;;
        compose-2c)
            log "starting server-alice"
            "${COMPOSE[@]}" up -d server-alice
            sleep 5
            log "waiting for alice events_done"
            wait_for_path server-alice /watch_alice/.events_done 300
            log "starting client-bob"
            "${COMPOSE[@]}" up -d client-bob
            log "waiting for bob events_done"
            wait_for_path client-bob /watch_bob/.events_done 300
            ;;
        compose-3c)
            log "starting server"
            "${COMPOSE[@]}" up -d server
            sleep 5
            log "starting client-alice"
            "${COMPOSE[@]}" up -d client-alice
            log "waiting for client-alice events_done"
            wait_for_path client-alice /watch_alice/.events_done 300
            log "starting client-bob"
            "${COMPOSE[@]}" up -d client-bob
            log "waiting for client-bob events_done"
            wait_for_path client-bob /watch_bob/.events_done 300
            ;;
    esac
}

min_generation_phase_a() {
    case "${COMPOSE_SPEC}" in
        compose-1c) echo 2 ;;
        *) echo 4 ;;
    esac
}

min_generation_phase_b() {
    case "${COMPOSE_SPEC}" in
        compose-1c) echo 4 ;;
        *) echo 10 ;;
    esac
}

log "building musl binaries (host target/ + cargo cache mount)"
bash "${REPO_ROOT}/scenario/scripts/build-musl.sh"

log "building runtime image"
if [[ "${COMPOSE_BUILD_PULL:-0}" == "1" ]]; then
    "${COMPOSE[@]}" build --pull
else
    "${COMPOSE[@]}" build
fi

bring_up

log "waiting for server generation to catch up to phase A"
wait_for_generation "$(min_generation_phase_a)" 180

log "waiting for server generation to settle before checkpoint A"
wait_for_generation_stable

for user in "${CLIENT_USERS[@]}"; do
    wait_for_server_mirror_dir "/srv/sinkd/watch_${user}" 240
done

log "checkpoint A"
for user in "${CLIENT_USERS[@]}"; do
    snapshot_diff "${user}" "/watch_${user}" A
done

log "phase B: second wave of events"
if [[ "${COMPOSE_SPEC}" == compose-1c ]]; then
    user_filter=alice run_phase_b
else
    run_phase_b
fi

log "settling for phase B"
sleep 20

log "waiting for server generation to catch up to phase B"
wait_for_generation "$(min_generation_phase_b)" 180

log "waiting for server generation to settle before checkpoint B"
wait_for_generation_stable

log "checkpoint B"
for user in "${CLIENT_USERS[@]}"; do
    snapshot_diff "${user}" "/watch_${user}" B
done

log "compose scenario OK"
