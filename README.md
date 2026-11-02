# ![image](assets/imgs/sinkd-black-txt.png)

## _Don't get out of `sinkd`_ ⚓︎

_**Everything and the kitchen sink**_  

## Deployable Cloud

_True Privacy_  
I believe your files should stay with you **always**.  

- **No** third party eyes
- **No** privacy policies
- **No** tradeoffs

Given the pleathora of cloud providers and the frequent data breaches of large companies 
I created `sinkd` to give the power back to the user. With an old machine, or a spare 
Raspberry Pi, you can create your own __local__ cloud! The purpose and goal of this program
is to get out of the users way while reducing the overhead of keeping up environments 
across many computers. 

One machine is the hub. The others watch folders and push. Zenoh carries the control messages over mutual TLS. libreflect copies the files over a short-lived TCP connection. Windows is WSL only.

Build needs the libreflect package (`reflect.pc`, `/usr/include/reflect.h`, `/usr/lib/libreflect.a`) and, on Arch, the `rust-musl` package. On macOS the build looks for `libreflect.a` in Homebrew's lib directory, `/usr/local/lib`, or `$REFLECT_LIB_DIR`. `cargo build --release` produces a static musl binary with libreflect, lz4, and zstd inside it. See [docs/protocol.md](docs/protocol.md) and [docs/encryption.md](docs/encryption.md).

### Try it on two machines

```bash
# machine A
cargo run -p sinkd-srv -- --sync-root ~/sinkd-data

# machine B, after the server prints a 6-digit code
cargo run -p sinkd -- pair <A-hostname-or-ip>
cargo run -p sinkd -- --server <A-hostname-or-ip> --anchor ~/Documents
```

Both stay in the foreground and log to stderr. Ctrl-C stops them. `--config` points at a toml file so a test does not need `/etc`.

### Installation

```bash
curl -fsSL https://sinkd.octoworks.shop/install.sh | sh
```

That is the release build. The site is [sinkd.octoworks.shop](https://sinkd.octoworks.shop).

**Linux:** `pkg/debian/create.sh` and `pkg/arch/create.sh` package the static musl build. Nothing else is required at runtime. `just test-docker` runs the sync scenarios in Alpine.

**macOS:** `brew install --head ./pkg/homebrew/sinkd.rb` after `libreflect.a` is installed.

systemd user unit: `pkg/systemd/sinkd.service`. systemd system unit: `pkg/systemd/sinkd-srv.service`. launchd plists are in `pkg/launchd/`.

### Workspace

| Crate | Role |
|-------|------|
| `sinkd-reflect-sys` | bindgen against the installed `reflect.h`, static link of `libreflect.a` |
| `sinkd-reflect` | safe wrapper around a reflect job |
| `sinkd-proto` | postcard messages and Zenoh key names |
| `sinkd-core` | config, pairing certs, paths |
| `sinkd-client` | `sinkd` |
| `sinkd-server` | `sinkd-srv` |
