# shellcheck shell=bash
# Shared SSH bootstrap for compose scenarios (sourced by entrypoint.sh).
setup_ssh() {
    if [[ ! -d /run/sinkd-ssh ]]; then
        log "ssh: /run/sinkd-ssh not mounted; skipping"
        return 0
    fi
    mkdir -p /root/.ssh /var/run/sshd
    chmod 700 /root/.ssh
    install -m 600 /run/sinkd-ssh/id_ed25519 /root/.ssh/id_ed25519
    install -m 644 /run/sinkd-ssh/id_ed25519.pub /root/.ssh/id_ed25519.pub
    install -m 600 /run/sinkd-ssh/authorized_keys /root/.ssh/authorized_keys
    : > /root/.ssh/known_hosts
    for host in ${SSH_PEERS:-}; do
        ssh-keyscan -H "${host}" >> /root/.ssh/known_hosts 2>/dev/null || true
    done
    if ! pgrep -x sshd >/dev/null 2>&1; then
        /usr/sbin/sshd
        log "ssh: sshd started"
    fi
}
