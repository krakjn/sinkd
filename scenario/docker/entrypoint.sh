#!/bin/sh
# Pair, then watch. Retries until the server is up. The code is the server's --pair-code.
set -eu
state=/var/lib/sinkd
run=/run/sinkd
mkdir -p "$state" "$run" /home/tony/sync /srv/share
i=0
while ! sinkd --state-dir "$state" pair server --code "${SINKD_CODE:-424242}" --pair-port 7449; do
    i=$((i + 1))
    if [ "$i" -gt 60 ]; then
        echo "pair failed" >&2
        exit 1
    fi
    sleep 0.5
done
exec sinkd --state-dir "$state" --runtime-dir "$run" --server server \
    --anchor /home/tony/sync --share /srv/share
