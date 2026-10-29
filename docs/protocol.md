# sinkd protocol

Control is Zenoh over mutual TLS. File bytes are a plain TCP connection with libreflect on both ends. There is no rsync and no SSH on the normal path. `sinkd ssh` remains a one-shot fallback.

Default ports, all bound by the server: Zenoh `7447`, data `7448`, pair `7449`.

## Pairing

The server prints a 6-digit code (`--pair-code` pins it for tests). The client connects to the pair port and exchanges length-prefixed frames (u32 big-endian length, then bytes):

1. client id
2. server `ready` (the code is not sent on the socket)
3. the code the operator typed
4. server `ok` or `no`
5. server id, CA pem, client cert pem, client key pem, 32-byte secret

Later requests carry that secret. The server compares it in constant time.

## Sync

The client queries `sinkd/v1/<server_id>/rpc` with a postcard `Request`. A grant is a single-use 16-byte token, a 30s lease, and the data port. The client writes the token, then both sides run libreflect on the socket (client sends on push, server sends on pull). The server bumps that anchor's generation only after a successful push and publishes `sinkd/v1/<server_id>/events/<anchor_key>`.

A push whose basis is not the anchor head is rejected. The client keeps dirty files under its state directory (`conflicts/<anchor_key>/`) and pulls. It does not push again on its own.

A client with no basis file pulls with delete off and update-only on, then pushes.

`anchor_id` is the FNV-1a hex of the canonical path. Personal trees live at `{sync_root}/{user}/{anchor_id}/`. Shared trees live at `{sync_root}/share/{anchor_id}/`.

Stop and reload go over a Unix socket, `$XDG_RUNTIME_DIR/sinkd.sock` or `sinkd-srv.sock`, not over Zenoh.
