# How to use sinkd

One computer is the hub. The others watch folders and keep them in step with that hub. Nothing here calls a cloud account.

You need two programs:

- `sinkd-srv` on the hub. It stores the files.
- `sinkd` on each other machine. It watches folders.

Both stay in the foreground and print logs to the terminal. Ctrl-C stops the one in that terminal.

## Install

```bash
curl -fsSL https://sinkd.octoworks.shop/install.sh | sh
```

That downloads the latest release and checks the checksum. Debian and Ubuntu get the `.deb`. Everywhere else gets a tarball in `/usr/local/bin`. Set `PREFIX` to put the binaries somewhere else, or `SINKD_VERSION=vX` to pin a tag.

The release binaries are static. They do not need libreflect installed to run.

Building from source does. On Linux, install the libreflect package so `pkg-config reflect` works. On macOS, put `libreflect.a` and `reflect.h` where Homebrew keeps libraries, or in `/usr/local`, or point `REFLECT_LIB_DIR` at the directory that contains `libreflect.a`. Then:

```bash
cargo build --release
```

The binaries are `target/x86_64-unknown-linux-musl/release/sinkd` and `sinkd-srv` on Linux. On Arch, install the `rust-musl` package first. macOS is not a musl build: `brew install --head ./pkg/homebrew/sinkd.rb` after libreflect is installed. Windows is WSL only.

## First run, two machines

On the hub:

```bash
sinkd-srv --sync-root ~/sinkd-data
```

It listens on every interface. The ports are 7447 (control), 7448 (file data), and 7449 (pairing). When a client asks to pair, the hub prints a 6-digit code.

On the other machine, in a second terminal, type that code when asked:

```bash
sinkd pair 192.168.1.20
```

Use the hub's hostname or IP. Pairing is once per machine. Then start watching a folder:

```bash
sinkd --server 192.168.1.20 --anchor ~/Documents
```

Create or edit a file under `~/Documents`. It shows up on the hub under `~/sinkd-data/`, inside a directory named for your user. Another paired client watching the same folder path, as the same user, receives that change.

`--share` is the other kind of folder. A share is one tree for every client, not one tree per user:

```bash
sinkd --server 192.168.1.20 --anchor ~/Documents --share ~/Shared
```

The hub keeps shares under `~/sinkd-data/share/`.

A file written on a client is pushed. A file that arrives from the hub is pulled. Deleting a file on one side deletes it on the other after the sync. Empty directories can be left behind.

## When two edits collide

The hub is the source of truth. If your copy is behind and you also changed files, sinkd does not overwrite your edits. It copies those files out of the synced folder into:

```text
~/.local/state/sinkd/conflicts/
```

Then it pulls the hub's version into the folder you are watching. Look in `conflicts/` for the copy that was set aside. The name ends in `.sinkd-conflict-<hostname>-<time>`.

## Config files

Flags override the file. The file overrides the defaults.

Hub, `~/.config/sinkd/server.toml`:

```toml
sync_root = "/srv/sinkd"
bind = "0.0.0.0"
zenoh_port = 7447
data_port = 7448
pair_port = 7449
```

Client, `~/.config/sinkd/client.toml`:

```toml
server = "192.168.1.20"
anchors = ["~/Documents"]
shares = ["~/Shared"]
```

`--config /path/to/file.toml` uses a different file. After a successful `pair`, the client remembers the hub, so later runs can omit `--server` if it was saved.

State (keys, the remembered hub, what has already synced) lives in `~/.local/state/sinkd` for a client and `~/.local/state/sinkd-srv` for the hub. `SINKD_STATE_DIR` overrides that. `~/` in paths is your home directory.

## Leave it running

The systemd units in `pkg/systemd/` are thin wrappers. The client unit is a user service (`sinkd`). The hub unit is a system service (`sinkd-srv`) running as user `sinkd`, with the store at `/srv/sinkd` and the config at `/etc/sinkd/server.toml`. macOS plists are in `pkg/launchd/`.

The hub prints the 6-digit pair code when a client connects, on stderr. Under systemd that goes to the journal, not the terminal. Start `sinkd pair` on the other machine, then on the hub:

```bash
journalctl -u sinkd-srv -f
```

Look for `pair code 424242 for client …` and type that code into the client.

`sinkd stop` and `sinkd-srv stop` ask the running process to exit. `reload` does not pick up new folders yet. Stop it and start it again.

## One copy, without the hub

```bash
sinkd ssh user@other-host:/tmp/backup/ --anchor ~/Documents
```

That is a single libreflect copy over SSH. It does not pair, watch, or use the hub. Add `--delete` to remove files at the destination that are gone locally.

## What this is not

Traffic stays on your LAN. The TLS check does not verify the peer's name, so do not put the hub on the public internet as-is. Encrypt the hub disk (LUKS or fscrypt on the sync store) if you want the files unreadable when the machine is off. sinkd does not encrypt each file itself.
