#!/bin/sh
# Starts the Cloudflare WARP proxy (usque), then the server.
#
# Everything the server sends to Microsoft goes through a local SOCKS5 proxy into Cloudflare WARP,
# so Microsoft sees Cloudflare's address and not this host's. The server checks that on its own
# (Cloudflare's trace page, through the proxy) and refuses to call Microsoft until it is confirmed.
#
# Why usque and not Cloudflare's own client: warp-svc manages the host's firewall (nftables) even in
# proxy mode, and a container without NET_ADMIN (Railway) cannot give it that, so it tore the tunnel
# down every few seconds ("FirewallRepairFailed"). usque speaks the same protocol (MASQUE) entirely
# in user space: no firewall, no root, no special capability.
set -eu

log() { echo "[entrypoint] $*"; }

if [ -n "${XAU_EGRESS_PROXY:-}" ]; then
  log "using the egress proxy given in XAU_EGRESS_PROXY"
elif [ "${XAU_WARP:-on}" = "off" ]; then
  log "WARP is off (XAU_WARP=off): calls to Microsoft will be refused"
else
  WARP_PORT="${WARP_PROXY_PORT:-40000}"
  DATA="${XAU_DATA_DIR:-/data}"
  CONFIG="${DATA}/warp-usque.json"
  mkdir -p "${DATA}"

  register() {
    log "registering a WARP device"
    (umask 077 && usque -c "${CONFIG}" register --accept-tos --name xau-web 2>&1 | sed 's/^/[warp] /') || true
  }

  # One WARP device, registered once and kept on the volume (registering on every deploy would
  # pile up devices at Cloudflare).
  [ -s "${CONFIG}" ] || register

  # HTTP/2 over TCP by default: a container host's UDP is not always kind to QUIC.
  # XAU_WARP_TRANSPORT=quic switches to HTTP/3.
  set -- socks -b 127.0.0.1 -p "${WARP_PORT}" -d 1.1.1.1 -d 1.0.0.1 --always-reconnect -k 15s
  if [ "${XAU_WARP_TRANSPORT:-http2}" = "quic" ]; then
    transport="HTTP/3"
  else
    transport="HTTP/2"
    set -- "$@" --http2
  fi
  log "starting WARP (usque over ${transport}, proxy 127.0.0.1:${WARP_PORT})"
  (
    while true; do
      [ -s "${CONFIG}" ] || register
      usque -c "${CONFIG}" "$@" 2>&1 | sed -u 's/^/[warp] /'
      echo "[entrypoint] WARP proxy exited; starting it again in 3 s"
      sleep 3
    done
  ) &

  i=0
  until nc -z 127.0.0.1 "${WARP_PORT}" 2> /dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 60 ]; then
      log "the WARP proxy did not come up in 60 s; the server will refuse calls to Microsoft until it does"
      break
    fi
    sleep 1
  done
  export XAU_EGRESS_PROXY="socks5h://127.0.0.1:${WARP_PORT}"
fi

exec /usr/local/bin/xau-web
