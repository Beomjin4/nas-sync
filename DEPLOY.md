# Deployment & First Use

**English** | [한국어](./DEPLOY.ko.md)

This walks you through running the server on a Synology NAS and installing the
plugin in an Obsidian vault.

---

## Server on Synology (Container Manager)

### Path A — SSH + docker compose (recommended; faster iteration)

Enable SSH on DSM (`Control Panel → Terminal & SNMP → Enable SSH`), then:

```bash
ssh you@<nas-ip>

sudo mkdir -p /volume1/docker/obsidian-nas
sudo chown "$USER" /volume1/docker/obsidian-nas
cd /volume1/docker

git clone https://github.com/Beomjin4/nas-sync.git obsidian-nas
cd obsidian-nas

cp .env.example .env
# Generate real secrets in-place:
sed -i "s|^ONS_JWT_SECRET=.*|ONS_JWT_SECRET=$(openssl rand -hex 48)|" .env
sed -i "s|^ONS_PAIRING_CODE=.*|ONS_PAIRING_CODE=$(openssl rand -hex 8)|" .env

# Note the pairing code — you'll need it on the client.
grep ONS_PAIRING_CODE .env

# Synology Docker doesn't auto-create bind-mount sources, and the container
# runs as uid 1000. Create data/ owned by 1000 before the first start:
mkdir -p data
sudo chown -R 1000:1000 data

sudo docker compose up -d --build
sudo docker compose logs -f obsidian-nas
```

First build takes a few minutes (musl Rust compile in a builder stage). Once
it's up:

```bash
curl http://localhost:8080/health
# {"service":"obsidian-nas-server","status":"ok"}
```

### Path B — Container Manager UI

DSM 7.2+ Container Manager supports compose projects:

1. Container Manager → Project → Create
2. **Name**: `obsidian-nas`
3. **Path**: `/volume1/docker/obsidian-nas` (must contain `docker-compose.yml` **and** your `.env`)
4. **Source**: "Use existing docker-compose.yml"
5. Build → Start

You can still tail logs with `sudo docker compose logs -f obsidian-nas` over SSH.

### Where data lives

Everything sits under `/volume1/docker/obsidian-nas/data/`:

```
data/
├── vault/        actual files (this mirrors your Obsidian vault)
├── trash/        soft-deleted files, kept for 30 days
├── conflicts/    losing versions from B+ conflict policy
└── meta.db       SQLite: files / devices / audit / conflicts / trash
```

Back up the whole `data/` directory and you've backed up the server.

### Common gotchas

- **Port 8080 already in use**: change `"8080:8080"` in `docker-compose.yml` to e.g. `"8089:8080"` and use `:8089` in the plugin.
- **Permission denied on data/**: the container runs as root by default (Synology ACLs often block unprivileged uids). If you hardened it with `user:` in docker-compose.yml, make sure `data/` is owned by that uid.
- **Builds fail with network error**: cargo needs to fetch crates from crates.io once. Make sure the NAS has internet during the first build.

---

## Plugin (Obsidian on Mac)

### 1. Drop the plugin into your vault

From this repo on your Mac:

```bash
VAULT="/path/to/your/vault"
mkdir -p "$VAULT/.obsidian/plugins/nas-sync"
cp plugin/manifest.json plugin/main.js "$VAULT/.obsidian/plugins/nas-sync/"
```

(Replace `/path/to/your/vault` with your actual vault path.)

### 2. Enable in Obsidian

1. Obsidian → **Settings → Community plugins**
2. If "Restricted mode" is on, turn it off
3. Click the refresh icon next to "Installed plugins"
4. Find **NAS Sync** and toggle it on

### 3. Pair

1. **Settings → NAS Sync** (left sidebar)
2. **Server URL**: `http://<nas-ip>:8080` — e.g. `http://192.168.1.10:8080`
3. **Device name**: e.g. `MacBook`
4. **Pairing code**: paste the `ONS_PAIRING_CODE` from the NAS
5. Click **Pair this device**

You should see "Paired with NAS". The pairing code is wiped from settings after success.

The **first device** paired uploads its whole vault to the NAS; devices
paired later receive that vault on first connect.

> ⚠ From the **second device onward**, pair into an **empty vault**: local
> files at paths that already exist on the server are overwritten by the
> server's version during the initial sync.

### 4. Try a sync

Create or edit a note. Within 5 seconds (debounce window):

```bash
# on the NAS
sudo docker compose logs --tail 30 obsidian-nas
# look for a line like:  PUT /file/notes/foo.md
ls /volume1/docker/obsidian-nas/data/vault/
# the note should appear here
```

### Troubleshooting

- **"Pairing failed: HTTP 401"** → wrong pairing code, or `ONS_PAIRING_CODE` not set on the server.
- **Plugin doesn't show up in Obsidian** → check `<vault>/.obsidian/plugins/nas-sync/` contains both `manifest.json` **and** `main.js`. Refresh the plugin list.
- **Pair succeeds but nothing syncs** → open Obsidian's developer console (Cmd+Opt+I → Console) and look for `[nas-sync]` log lines. Check the NAS server logs too.
- **WebSocket keeps reconnecting** → server URL is right but `/sync` isn't reachable. If you're behind a reverse proxy, make sure it forwards `Upgrade: websocket`.
