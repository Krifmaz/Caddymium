# Caddymium, all in one: the panel plus Caddy in a single image. The panel
# starts Caddy and keeps it running (the image sets
# CADDYMIUM_MANAGE_CADDY_DEFAULT=true). Set CADDYMIUM_MANAGE_CADDY=false to
# use this image as the panel of the two-container layout
# (docker-compose.split.yml); the pre-rename CADDYHUB_* names still work.
#
# Build stages run on the build machine's architecture and cross-compile
# with cargo-zigbuild (zig as the C compiler/linker), so arm64 images don't
# need emulation.

FROM --platform=$BUILDPLATFORM node:24-alpine AS ui
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN mkdir -p ../backend/ui && npm run build

FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build
ARG VERSION=dev
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends python3-pip && rm -rf /var/lib/apt/lists/* \
    && pip3 install --break-system-packages --no-cache-dir "cargo-zigbuild==0.23.4" "ziglang<0.15"
WORKDIR /src/backend
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl > /target ;; \
      arm64) echo aarch64-unknown-linux-musl > /target ;; \
      *) echo "unsupported TARGETARCH $TARGETARCH" >&2; exit 1 ;; \
    esac && rustup target add "$(cat /target)"
# Dependencies first, so they are cached across source changes.
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir -p src ui/dist && echo 'fn main() {}' > src/main.rs \
    && cargo zigbuild --release --locked --target "$(cat /target)" \
    && rm -rf src target/*/release/deps/caddymium* target/*/release/caddymium*
COPY backend/ ./
COPY docs/releases /src/docs/releases
COPY --from=ui /src/backend/ui/dist ./ui/dist
RUN CADDYMIUM_VERSION="$VERSION" cargo zigbuild --release --locked --target "$(cat /target)" \
    && cp "target/$(cat /target)/release/caddymium" /caddymium

# Caddy with the default plugins, from the official download service (what
# `caddy add-package` uses). More plugins can be added later from the panel.
FROM --platform=$BUILDPLATFORM alpine:3 AS caddy
ARG TARGETARCH
ARG CADDY_PLUGINS="github.com/caddy-dns/cloudflare github.com/caddy-dns/porkbun github.com/caddy-dns/duckdns github.com/caddy-dns/namecheap github.com/caddy-dns/route53 github.com/caddy-dns/digitalocean github.com/caddy-dns/hetzner github.com/caddy-dns/desec github.com/caddy-dns/ovh github.com/caddy-dns/gandi github.com/caddy-dns/acmedns github.com/mholt/caddy-ratelimit github.com/hslatman/caddy-crowdsec-bouncer"
RUN apk add --no-cache curl && q=""; for p in $CADDY_PLUGINS; do q="$q&p=$p"; done \
    && curl -fsSL --retry 3 -o /caddy "https://caddyserver.com/api/download?os=linux&arch=${TARGETARCH}${q}" \
    && chmod 0755 /caddy && test "$(stat -c %s /caddy)" -gt 10000000

FROM alpine:3
RUN apk add --no-cache ca-certificates tzdata mailcap
COPY --from=build /caddymium /usr/local/bin/caddymium
COPY --from=caddy /caddy /usr/bin/caddy
ENV CADDYMIUM_MANAGE_CADDY_DEFAULT=true \
    XDG_DATA_HOME=/data/caddy-data \
    XDG_CONFIG_HOME=/data/caddy-config
VOLUME /data
EXPOSE 80 443 443/udp 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["caddymium", "-healthcheck"]
ENTRYPOINT ["caddymium"]
