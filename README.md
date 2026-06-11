# NAS Sync for Obsidian

**English** | [한국어](./README.ko.md)

Self-hosted, real-time vault synchronization on your own hardware. A single
Rust binary runs on your NAS; an Obsidian plugin keeps every device in sync.

## Features

- ⚡ **Near-real-time sync** — file changes propagate to all connected devices
  within ~5 seconds (debounced, WebSocket push)
- 🔁 **Multi-device** — macOS, Windows, Linux, Android, iOS (anywhere Obsidian
  community plugins run)
- 🛟 **Zero-data-loss conflicts** — concurrent edits never destroy a version:
  the latest write becomes active, the other is preserved server-side and
  resolvable from any device or the web console
- 🗑 **Trash with retention** — deletes are kept on the NAS for 30 days
  (configurable) and restorable with one click
- 📜 **Audit log** — every create / modify / delete / conflict is recorded
- 🖥 **Web admin console** — activity log, file search with preview, conflict
  resolution, trash restore, device management (English / Korean)
- 🪶 **Tiny footprint** — single static binary, ~15 MB container, SQLite,
  no background polling

## How it works

```
 Obsidian (Mac) ──┐                       ┌── Obsidian (phone)
                  │   HTTP (files, ETag)  │
                  ├──────────► NAS ◄──────┤
                  │      WebSocket        │
 Obsidian (PC) ───┘    (change events)    └── Web admin console
```

- The NAS holds the canonical copy of your vault plus `trash/`, `conflicts/`,
  and a SQLite index (`meta.db`).
- Each file carries a BLAKE3 ETag. Writes use compare-and-swap (`If-Match`);
  a stale write triggers the conflict flow instead of silently overwriting.
- Clients keep a persisted queue, so offline edits upload on reconnect.
  Files changed while Obsidian was closed are detected via an mtime/size
  baseline at startup.

### Conflict policy

When two devices change the same file in the same window:

1. The **latest write wins** and stays active — your flow is never blocked.
2. The losing version is **preserved on the server** (never synced around as
   duplicate files).
3. All devices get a conflict notice (status bar + notification). Resolve from
   the plugin's diff modal or the web console: *keep active*, *use other
   version*, or *keep both*.
4. Modify-vs-delete conflicts: **modify wins**, deletes never destroy edits.

## Quick start

### 1. Server (Docker)

```bash
git clone https://github.com/Beomjin4/nas-sync.git nas-sync && cd nas-sync

cp .env.example .env
# set ONS_JWT_SECRET, ONS_PAIRING_CODE, ONS_ADMIN_PASSWORD — see .env.example

mkdir -p data
docker compose up -d --build
curl http://localhost:8080/health   # → {"status":"ok", ...}
```

Prebuilt images are published to GHCR on each release — see
`docker-compose.yml` comments to use `ghcr.io/Beomjin4/nas-sync` instead of
building from source.

Synology / QNAP notes, reverse-proxy and TLS guidance: [DEPLOY.md](./DEPLOY.md).

### 2. Plugin

Until the plugin is in the community store, install manually:

1. Download `nas-sync.zip` from the [latest release](../../releases/latest)
2. Extract into `<your vault>/.obsidian/plugins/` (creating
   `plugins/nas-sync/` with `main.js` + `manifest.json`)
3. Obsidian → Settings → Community plugins → enable **NAS Sync**
4. In the plugin settings: enter your server URL and the pairing code from
   your `.env`, then **Pair this device**

**How onboarding works:** the **first device** you pair uploads its entire
vault to the NAS — that becomes the canonical copy. Every device paired
afterwards receives that vault immediately on first connect.

> ⚠ **Pair additional devices into an empty vault.** If the second device's
> vault already contains files at the same paths, they are **overwritten by
> the server's version** during the initial sync. Files at paths the server
> doesn't know are uploaded and kept, but same-path local versions on a
> newly paired device do not win. Back up first if in doubt.

### 3. Admin console

Open `http://<nas>:8080/admin` (or the **Open admin console** button in the
plugin settings). Sign in with `ONS_ADMIN_PASSWORD`.

## Configuration

All via environment variables (see [.env.example](./.env.example)):

| Variable | Default | Purpose |
|---|---|---|
| `ONS_JWT_SECRET` | — (required) | Signs device tokens |
| `ONS_PAIRING_CODE` | unset (pairing disabled) | Pre-shared code to enroll a device |
| `ONS_ADMIN_PASSWORD` | unset (console disabled) | Web console login |
| `ONS_TRASH_TTL_DAYS` | `30` | Days before trash is purged |
| `ONS_MAX_FILE_SIZE_MB` | `100` | Upload size limit |
| `ONS_BIND` | `0.0.0.0:8080` | Listen address |

## Security model

Designed for a **trusted LAN or VPN** (Tailscale/WireGuard work well):

- Device auth: pre-shared pairing code → per-device JWT; devices are
  individually revocable from the console
- Admin console: password + HttpOnly session cookie
- The server speaks plain HTTP — **put TLS in front** (reverse proxy) before
  exposing it beyond your LAN. Direct internet exposure is not recommended.

## ⚠ Scope

Built for **single-user, multi-device** use. Concurrent editing by multiple
people may work, but the conflict policy assumes conflicts are occasional.
For real-time collaborative editing, consider
[Self-hosted LiveSync](https://github.com/vrtmrz/obsidian-livesync) or
[Relay](https://relay.md) instead.

## Development

```
server/   Rust (axum + tokio + sqlx/SQLite + blake3)
plugin/   TypeScript (esbuild), no editor injection — vault events only
```

```bash
# server
cd server && cargo test && cargo run

# plugin
cd plugin && npm install && npm run dev
```

Releases are automated: pushing a `v*` tag builds the server binaries, the
GHCR image, and the plugin bundle (see `.github/workflows/release.yml`).

## License

[MIT](./LICENSE)
