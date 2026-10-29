FROM alpine:3
COPY bin/sinkd bin/sinkd-srv /usr/local/bin/
COPY entrypoint.sh /usr/local/bin/sinkd-client-entry
RUN chmod +x /usr/local/bin/sinkd /usr/local/bin/sinkd-srv /usr/local/bin/sinkd-client-entry
ENTRYPOINT ["sinkd-client-entry"]
