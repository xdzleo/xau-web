# The web version as a server (Railway, or any container host).
#
#   docker build -t xau-web .
#   docker run -p 8080:8080 -v xau-data:/data \
#     -e PORT=8080 -e XAU_PUBLIC_HOSTS=rewards.example.com -e XAU_KEY=<32+ chars> \
#     -e WARP_TEAM=<team> -e WARP_CLIENT_ID=<id> -e WARP_CLIENT_SECRET=<secret> xau-web
#
# One container holds the server, the headless Chromium that runs Microsoft's sign-in page, and
# Cloudflare WARP in proxy mode, through which everything meant for Microsoft leaves (see
# docs/DEPLOY-RAILWAY.md). Without the WARP variables the server refuses to talk to Microsoft.

# ---- build -------------------------------------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /src
# The repository's .cargo/config.toml asks for x86-64-v3 (a Windows desktop tuning); a server
# may run on an older CPU, so this build targets the baseline.
ENV RUSTFLAGS="-C target-cpu=x86-64"
COPY . .
RUN cargo build --release -p xau-web \
 && strip target/release/xau-web \
 && cp target/release/xau-web /xau-web

# ---- run ---------------------------------------------------------------------------------
FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl gnupg dbus netcat-openbsd procps \
      chromium fonts-liberation fonts-noto-color-emoji \
 && curl -fsSL https://pkg.cloudflareclient.com/pubkey.gpg \
      | gpg --yes --dearmor -o /usr/share/keyrings/cloudflare-warp-archive-keyring.gpg \
 && echo "deb [signed-by=/usr/share/keyrings/cloudflare-warp-archive-keyring.gpg] https://pkg.cloudflareclient.com/ bookworm main" \
      > /etc/apt/sources.list.d/cloudflare-client.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends cloudflare-warp \
 && rm -rf /var/lib/apt/lists/*

COPY --from=build /xau-web /usr/local/bin/xau-web
COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# A container runs as root, where Chromium's own sandbox cannot start.
ENV XAU_CHROMIUM=/usr/bin/chromium \
    XAU_CHROMIUM_NO_SANDBOX=1 \
    XAU_DATA_DIR=/data
# The data folder is a volume, but not declared here: Railway refuses VOLUME in a Dockerfile. Its volume
# (mount path /data, see docs/DEPLOY-RAILWAY.md) is made in the Railway panel; elsewhere use -v xau-data:/data.
EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]
