# syntax=docker/dockerfile:1.7
# Runtime image for compose scenarios. Musl binaries built via scenario/scripts/build-musl.sh.

FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        rsync \
        openssh-client \
        openssh-server \
        ca-certificates \
        coreutils \
        findutils \
        bash \
        procps \
 && rm -rf /var/lib/apt/lists/*

# Sinkd client configs resolve per Unix account (see `users` in system config).
RUN useradd -m -s /bin/bash alice \
 && useradd -m -s /bin/bash bob

COPY pkg/artifacts/sinkd_*.deb /tmp/
RUN dpkg -i /tmp/sinkd_*.deb && rm /tmp/sinkd_*.deb

COPY scenario/compose/sshd_config /etc/ssh/sshd_config.d/sinkd-scenario.conf
COPY scenario/compose/ssh-setup.sh /usr/local/lib/sinkd/ssh-setup.sh
COPY scenario/compose/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/lib/sinkd/ssh-setup.sh

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
