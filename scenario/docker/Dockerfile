FROM alpine:3
# openssh and reflect (when bin/ has it) back the `sinkd ssh` check.
RUN apk add --no-cache openssh
COPY bin/ /usr/local/bin/
COPY entrypoint.sh /usr/local/bin/sinkd-client-entry
RUN chmod +x /usr/local/bin/* \
    && ssh-keygen -A \
    && adduser -D tony \
    && sed -i 's/^tony:!/tony:*/' /etc/shadow
ENTRYPOINT ["sinkd-client-entry"]
