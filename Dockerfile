# The web version as a server (Railway, or any container host).
#
#   docker build -t xau-web .
#   docker run -p 8080:8080 -v xau-data:/data \
#     -e PORT=8080 -e XAU_PUBLIC_HOSTS=rewards.example.com -e XAU_PASSWORD=<one,or,several> xau-web
#
# One container holds the server, the headless Chromium that runs Microsoft's sign-in page, and
# a Cloudflare WARP proxy, through which everything meant for Microsoft leaves (see
# docs/DEPLOY-RAILWAY.md). If WARP is not up, the server refuses to talk to Microsoft.

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
      ca-certificates curl unzip netcat-openbsd procps \
      chromium fonts-liberation fonts-noto-color-emoji \
 && rm -rf /var/lib/apt/lists/*

# usque: Cloudflare WARP's MASQUE protocol in user space, as a local SOCKS5 proxy (see
# docker/entrypoint.sh for why not Cloudflare's own client). Pinned, checksum verified.
ARG USQUE_VERSION=4.2.1
ARG USQUE_SHA256=4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313
RUN curl -fsSL -o /tmp/usque.zip \
      "https://github.com/Diniboy1123/usque/releases/download/v${USQUE_VERSION}/usque_${USQUE_VERSION}_linux_amd64.zip" \
 && echo "${USQUE_SHA256}  /tmp/usque.zip" | sha256sum -c - \
 && unzip -o /tmp/usque.zip usque -d /usr/local/bin \
 && chmod +x /usr/local/bin/usque \
 && rm /tmp/usque.zip

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
