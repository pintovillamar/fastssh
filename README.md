# FastSSH

A fast, minimal terminal and SSH client: one Rust binary that serves a browser
interface, with a desktop app planned on the same core.

Status: early. Today it opens a local shell in the browser. SSH, logins and the
encrypted vault come next.

## Layout

- `crates/core` — shells and (soon) SSH, storage and the vault. No web code.
- `crates/server` — the `fastssh` binary: HTTP, websockets, embedded interface.
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

## Security

There is no login yet. The server only listens on localhost by default and
refuses websockets from other sites. Do not expose it on a network: anyone who
can reach the port gets a shell as you.
