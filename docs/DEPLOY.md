# Deploying FastSSH

FastSSH is one program and one database file. Pick either the plain binary
with systemd or the container image, then put https in front of it.

Whichever way you run it, the first person to open the page creates the admin
account, so do that yourself straight after starting it.

## Option 1: binary and systemd

Download the archive for your machine from the
[releases page](https://github.com/pintovillamar/fastssh/releases)
(`x86_64` for most servers, `aarch64` for ARM ones such as a Raspberry Pi) and
install it:

```sh
tar -xzf fastssh-v*-x86_64-unknown-linux-gnu.tar.gz
cd fastssh-v*-x86_64-unknown-linux-gnu
sudo install -m 755 fastssh /usr/local/bin/fastssh
sudo install -m 644 fastssh.service /etc/systemd/system/fastssh.service
sudo systemctl daemon-reload
sudo systemctl enable --now fastssh
```

It now listens on `127.0.0.1:7422` and keeps its database in
`/var/lib/fastssh`. The service runs as an unprivileged user that can write
nowhere else. The binaries need glibc 2.35 or newer (Ubuntu 22.04, Debian 12
and later).

Settings go in `/etc/fastssh.env`, one per line, followed by
`sudo systemctl restart fastssh`:

```sh
FASTSSH_PUBLIC_URL=https://ssh.example.com
# FASTSSH_ALLOW_SIGNUP=true
# FASTSSH_GOOGLE_CLIENT_ID=...
# FASTSSH_GOOGLE_CLIENT_SECRET=...
```

That file can hold a secret, so make it readable by root only:
`sudo chmod 600 /etc/fastssh.env`.

Logs: `journalctl -u fastssh -f`. To update, replace the binary and restart
the service.

## Option 2: container

```sh
docker run -d --name fastssh --restart unless-stopped \
  -p 127.0.0.1:7422:7422 \
  -v fastssh-data:/data \
  -e FASTSSH_PUBLIC_URL=https://ssh.example.com \
  ghcr.io/pintovillamar/fastssh:latest
```

The database lives in the `fastssh-data` volume. `latest` is the newest
release; use a tag such as `0.2` to stay on one release line. To build the image yourself
instead, run `docker build -t fastssh .` in a checkout of the repository.

If you mount a host folder on `/data` instead of a named volume, it has to be
writable by user id 10001.

## https in front

Do not expose port 7422 to a network directly: without https, passwords and
terminal sessions travel unencrypted. Put a reverse proxy in front and tell
FastSSH its public address with `FASTSSH_PUBLIC_URL`.

With [Caddy](https://caddyserver.com), which gets certificates by itself, the
whole configuration is:

```
ssh.example.com {
    reverse_proxy 127.0.0.1:7422
}
```

With nginx, the terminal needs websocket upgrades passed through:

```nginx
location / {
    proxy_pass http://127.0.0.1:7422;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 1d;
}
```

## Backups

Everything is in `fastssh.db` in the data folder (plus `fastssh.db-wal` and
`fastssh.db-shm` while the server runs). Copy all three, or stop the service
first and copy the one file. Saved secrets in a backup stay encrypted; they
are useless without the owner's passphrase.

## Making a release (maintainers)

1. Set the new version in the top-level `Cargo.toml` and run `cargo build` so
   `Cargo.lock` follows. Commit.
2. Tag it and push the tag:

   ```sh
   git tag v0.3.0
   git push origin main v0.3.0
   ```

The release workflow builds both server binaries and the desktop app
(AppImage and `.deb` for Linux, an installer for Windows), publishes the
GitHub release with checksums and pushes the container image. Before
publishing, it installs the Windows installer on a clean machine and checks
that the app starts. To rehearse without publishing,
run the workflow by hand from the Actions tab or push to a branch named
`release-dry-run`.
