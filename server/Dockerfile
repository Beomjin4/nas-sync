# syntax=docker/dockerfile:1.6

# ---------- builder ----------
# Some transitive deps require recent rustc. We use the floating `1-alpine`
# tag for now; once we know which exact version the NAS pulls we can pin it.
FROM rust:1-alpine AS builder

RUN apk add --no-cache musl-dev pkgconfig

WORKDIR /build

# Pre-compile dependencies in a separate layer so source-only edits stay cached.
COPY Cargo.toml ./
COPY Cargo.lock* ./
# Alpine's native target is already musl, so no explicit --target: this lets
# buildx produce both linux/amd64 and linux/arm64 images from the same file.
RUN mkdir src \
 && echo "fn main() {}" > src/main.rs \
 && cargo build --release \
 && rm -rf src target/release/.fingerprint/obsidian-nas-server-*

COPY src ./src
COPY migrations ./migrations

RUN cargo build --release \
 && cp target/release/obsidian-nas-server /tmp/server

# ---------- runtime ----------
FROM alpine:3.20

RUN apk add --no-cache ca-certificates tini wget

WORKDIR /app
COPY --from=builder /tmp/server /app/server

ENV ONS_BIND=0.0.0.0:8080 \
    ONS_DATA_DIR=/data \
    ONS_VAULT_PATH=/data/vault \
    ONS_TRASH_PATH=/data/trash \
    ONS_CONFLICTS_PATH=/data/conflicts \
    ONS_DB_PATH=/data/meta.db

# Run as root inside the container. Synology's ACL on /volume1 frequently
# blocks an unprivileged uid 1000 even when the directory is chowned, so
# letting the binary run as root is the most robust default. The container
# is otherwise isolated; if you want to drop privileges, add
# `user: "1000:1000"` in docker-compose.yml and ensure the host data/ is
# owned by that uid (and not blocked by ACLs).
EXPOSE 8080
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/sbin/tini", "--"]
CMD ["/app/server"]
