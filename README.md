# FastSSH

A fast, minimal terminal and SSH client. It runs two ways from the same code:
as a server you open in a browser, and as a desktop app.

Status: early. It has an encrypted vault for saved passwords and keys, SSH
sessions and local shells in tabs, host key checking, and accounts on the
server.

## Layout

- `crates/core` — SSH, local shells, SQLite storage and the vault. No web code.
- `crates/server` — the `fastssh` binary: accounts, JSON API, websockets,
  embedded interface. Also a library, which the desktop app embeds.
- `crates/desktop` — the desktop app (Tauri): a native window around the same
  server, running privately inside the app.
- `web` — the interface (Svelte + xterm.js), built into `web/dist`.

## Run

```sh
cd web && npm install && npm run build && cd ..
cargo run
```

Then open http://127.0.0.1:7422 and create the first account. That account is
the admin.

For interface work with hot reload, keep `cargo run` going and start
`npm run dev` in `web/`; open the address Vite prints.

## Release build

```sh
cd web && npm run build && cd ..
cargo build --release
```

`target/release/fastssh` contains the interface and needs nothing else.

Ready-made server binaries for Linux (x86_64 and ARM) and the desktop app are
attached to each [release](https://github.com/pintovillamar/fastssh/releases),
and a container image is published as `ghcr.io/pintovillamar/fastssh`.

## Desktop app

The desktop app needs no server and no account. It asks for a master password
the first time, which encrypts what you save, and its local shell is your own
computer's. Nothing is opened to the network.

Releases include an AppImage and a `.deb` for Linux. To run it from source
you need WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch, `libwebkit2gtk-4.1-dev` on
Debian and Ubuntu):

```sh
cd web && npm install && npm run build && cd ..
cargo run -p fastssh-desktop
```

To build the AppImage and `.deb` yourself:

```sh
cd crates/desktop && npm install && npx tauri build
```

Its data lives in `~/.local/share/io.github.pintovillamar.fastssh`, separate
from the server's. Plain `cargo build` and `cargo test` skip the desktop app,
so the server can be built without WebKitGTK.

## Options

Every flag also has an environment variable; `fastssh --help` lists them.

| Flag | What it does |
| --- | --- |
| `--listen 127.0.0.1:7422` | Address to listen on. |
| `--data-dir <dir>` | Where the database lives (default `~/.local/share/fastssh`). |
| `--public-url https://ssh.example.com` | The address people use to reach the server. Needed behind a reverse proxy and for Google sign-in. With https, cookies are marked Secure. |
| `--allow-signup` | Let anyone who can reach the server create an account. Off by default: only the first account can be created. |
| `--no-local-shell` | Do not offer the admin a shell on the server machine. |

To reach FastSSH from other machines, put it behind a reverse proxy that
provides https and pass `--public-url`. Without https, passwords and terminal
sessions cross the network unencrypted. [docs/DEPLOY.md](docs/DEPLOY.md) covers
running it as a systemd service or a container, with proxy examples.

### Sign in with Google

Create an OAuth client (type "Web application") in the Google Cloud console
with the redirect URI `<public-url>/api/auth/google/callback`, then set:

```sh
FASTSSH_GOOGLE_CLIENT_ID=...
FASTSSH_GOOGLE_CLIENT_SECRET=...
```

A Google sign-in is matched to an account by its verified email address. New
addresses get an account only when sign-ups are open (or no account exists
yet).

## Accounts and the vault

Signing in and unlocking the vault are separate:

- A **session** is a cookie that lasts 30 days.
- The **vault** holds saved passwords, private keys and key passphrases,
  encrypted with a key derived from your passphrase. That key is kept only in
  the server's memory while you are signed in.

Signing in with a password unlocks the vault at once. After a Google sign-in
or a server restart you are asked for the passphrase. For password accounts
the passphrase is the password; accounts that only use Google choose a vault
passphrase the first time.

There is no passphrase recovery. A forgotten passphrase means the saved
secrets are lost, by design: the server cannot decrypt them either.

The first account is the admin. Only the admin gets the local shell, because
it runs as the operating system user FastSSH runs as.

## Data

Everything is in one SQLite file, `fastssh.db`, in the data folder. Connection
names, hosts and usernames are stored as plain text; secrets are stored
encrypted (XChaCha20-Poly1305, key wrapped with Argon2id).

## On a phone

On touch devices the terminal shows a row of keys a phone keyboard lacks:
Esc, Tab, arrows, Home/End, PgUp/PgDn and a few symbols. Ctrl and Alt are
sticky: tap one, then the next key (from the row or the keyboard) is sent
with it.

## Tests

```sh
cargo test
cd web && npm test
```

The SSH tests run against a small SSH server started inside the test process.

## Security notes

- Requests from other websites are refused, and the session cookie is
  HttpOnly and SameSite.
- Five wrong passwords for one email block further attempts for five minutes.
- Signing out ends that session's open terminals.

## License

MIT. See [LICENSE](LICENSE).
