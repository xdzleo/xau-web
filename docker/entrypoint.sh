#!/bin/sh
# Starts Cloudflare WARP in proxy mode (when it is configured), then the server.
#
# Everything the server sends to Microsoft goes through WARP's local SOCKS5 proxy, so Microsoft
# sees Cloudflare's address and not this host's. The server checks that on its own (Cloudflare's
# trace page, through the proxy) and refuses to call Microsoft until it is confirmed.
set -eu

log() { echo "[entrypoint] $*"; }

if [ -n "${WARP_TEAM:-}" ] && [ -n "${WARP_CLIENT_ID:-}" ] && [ -n "${WARP_CLIENT_SECRET:-}" ]; then
  WARP_PORT="${WARP_PROXY_PORT:-40000}"
  log "configuring WARP (team ${WARP_TEAM}, proxy mode, port ${WARP_PORT})"

  # Headless enrollment with a Zero Trust service token (the token needs a "Service Auth" rule in
  # the team's device enrollment permissions).
  mkdir -p /var/lib/cloudflare-warp /run/dbus
  (
    umask 077
    cat > /var/lib/cloudflare-warp/mdm.xml <<EOF
<dict>
  <key>organization</key>
  <string>${WARP_TEAM}</string>
  <key>auth_client_id</key>
  <string>${WARP_CLIENT_ID}</string>
  <key>auth_client_secret</key>
  <string>${WARP_CLIENT_SECRET}</string>
  <key>service_mode</key>
  <string>proxy</string>
  <key>proxy_port</key>
  <integer>${WARP_PORT}</integer>
  <key>auto_connect</key>
  <integer>1</integer>
</dict>
EOF
  )

  rm -f /run/dbus/pid
  dbus-daemon --system --fork || log "dbus did not start (WARP may still work)"
  # warp-svc is kept alive: if it falls over, it is started again (it reads mdm.xml each time).
  (
    while true; do
      warp-svc >> /var/log/warp-svc.log 2>&1
      echo "[entrypoint] warp-svc exited (code $?); starting it again in 3 s"
      sleep 3
    done
  ) &
  sleep 3
  warp-cli --accept-tos status > /dev/null 2>&1 || true

  i=0
  until nc -z 127.0.0.1 "${WARP_PORT}" 2> /dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 90 ]; then
      log "the WARP proxy did not come up in 90 s; the server will refuse calls to Microsoft. warp-svc says:"
      tail -n 40 /var/log/warp-svc.log || true
      break
    fi
    sleep 1
  done

  # What WARP says, once now and then every minute, so the logs show why it is or is not up.
  log "warp-cli status: $(warp-cli --accept-tos status 2>&1 | tr '\n' ' ')"
  log "warp-svc log (last lines):"
  tail -n 25 /var/log/warp-svc.log 2> /dev/null | sed 's/^/[warp-svc] /' || true
  (
    seen=$(wc -l < /var/log/warp-svc.log 2> /dev/null || echo 0)
    n=0
    while true; do
      sleep 15
      # What warp-svc said since the last look: status changes, warnings and errors, without the
      # per-packet debug noise.
      now=$(wc -l < /var/log/warp-svc.log 2> /dev/null || echo 0)
      [ "$now" -lt "$seen" ] && seen=0
      if [ "$now" -gt "$seen" ]; then
        tail -n +"$((seen + 1))" /var/log/warp-svc.log | head -n "$((now - seen))" \
          | grep -E ' (WARN|ERROR) |Ipc Broadcast ResponseStatus|Disconnect|disconnect|Reason|closed|timed out|timeout' \
          | grep -v -E 'slog\.|time\.busy' | cut -c1-300 | tail -n 30 | sed 's/^/[warp-svc] /'
        seen=$now
      fi
      n=$((n + 1))
      if [ $((n % 4)) -eq 0 ]; then
        echo "[warp] $(warp-cli --accept-tos status 2>&1 | tr '\n' ' ') | proxy port ${WARP_PORT}: $(nc -z 127.0.0.1 "${WARP_PORT}" 2> /dev/null && echo listening || echo NOT listening)"
      fi
    done
  ) &
  export XAU_EGRESS_PROXY="socks5h://127.0.0.1:${WARP_PORT}"
else
  log "WARP is not configured (WARP_TEAM, WARP_CLIENT_ID, WARP_CLIENT_SECRET): calls to Microsoft will be refused"
fi

exec /usr/local/bin/xau-web
