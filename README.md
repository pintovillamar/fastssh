# FastSSH

A fast, minimal terminal and SSH client: one Rust binary that serves a browser
interface, with a desktop app planned on the same core.

Status: early. It opens SSH sessions and local shells in browser tabs, with
saved connections and host key checking. Logins and the encrypted vault come
next.

## Layout

- `crates/core` — SSH, local shells and SQLite storage. No web code.
- `crates/server` — the `fastssh` binary: JSON API, websockets, embedded interface.
- `web` — the interface (Svelte + xterm.js), built into `web/dist`.

## Run

```sh
cd web && npm install && npm run build && cd ..
cargo run
```

Then open http://127.0.0.1:7422.

For interface work with hot reload, keep `cargo run` going and start
`npm run dev` in `web/`; open the address Vite prints.

## Release build

```sh
cd web && npm run build && cd ..
cargo build --release
```

`target/release/fastssh` contains the interface and needs nothing else.

## Data

Saved connections and trusted host keys live in one SQLite file,
`~/.local/share/fastssh/fastssh.db`. Use `--data-dir` (or `FASTSSH_DATA_DIR`)
to put it elsewhere. No passwords or passphrases are stored: they are asked
for on each connect until the vault exists. Key files are read from the
machine FastSSH runs on.

## Tests

```sh
cargo test
```

## Security

There is no login yet. The server only listens on localhost by default and
refuses requests from other sites. Do not expose it on a network: anyone who
can reach the port gets a shell as you.
