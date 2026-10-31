// Checks the Wisp relay the way a browser's sign-in page uses it, against a running container:
//   node wisp-check.mjs ws://localhost:8080/wisp/ http://localhost:8080 "<cookie header>"
// The relay must refuse anything that is not a real host name, on a web port, over TCP, and (with
// WARP off, as in CI) must not open even a stream to a real name: nothing leaves except through
// Cloudflare.
import WebSocket from 'ws';

const [url, origin, cookie] = process.argv.slice(2);
const T_CONNECT = 1, T_CONTINUE = 3, T_CLOSE = 4;

function connectPacket(id, host, port, type = 1) {
  const h = Buffer.from(host);
  const b = Buffer.alloc(5 + 3 + h.length);
  b.writeUInt8(T_CONNECT, 0);
  b.writeUInt32LE(id, 1);
  b.writeUInt8(type, 5);
  b.writeUInt16LE(port, 6);
  h.copy(b, 8);
  return b;
}

const fail = (m) => {
  console.error('FAIL:', m);
  process.exit(1);
};

// Without the session cookie the upgrade is refused.
await new Promise((ok) => {
  const bare = new WebSocket(url, { headers: { origin } });
  bare.on('unexpected-response', (_req, res) => {
    if (res.statusCode !== 401) fail(`no cookie: expected 401, got ${res.statusCode}`);
    console.log('ok: no cookie -> 401');
    ok();
  });
  bare.on('open', () => fail('no cookie: the upgrade was accepted'));
  bare.on('error', () => {});
});

const ws = new WebSocket(url, { headers: { origin, cookie } });
const closes = new Map();
let first = null;
ws.on('message', (d) => {
  const b = Buffer.from(d);
  if (first === null) first = b;
  else if (b[0] === T_CLOSE) closes.set(b.readUInt32LE(1), b[5]);
});
await new Promise((ok, no) => {
  ws.on('open', ok);
  ws.on('error', no);
  setTimeout(() => no(new Error('open timeout')), 10000);
});
await new Promise((r) => setTimeout(r, 300));
if (!first || first[0] !== T_CONTINUE || first.readUInt32LE(1) !== 0 || first.readUInt32LE(5) < 1) fail(`the server must speak first with CONTINUE on stream 0, got ${first && first.toString('hex')}`);
console.log('ok: server speaks first (CONTINUE, stream 0, buffer', first.readUInt32LE(5) + ')');

const cases = [
  // A real name, any name, reaches the same wall every other stream does here: no confirmed
  // Cloudflare exit in CI (WARP is off). The allow-list is gone (see wisp.rs): a fixed list of
  // domains could never keep up with the sign-up's own vendor pages, whose bot-check domains
  // rotate through many short-lived aliases.
  [1, 'evil.example.com', 443, 1, 0x03, 'a real-looking name, not blocked by name any more'],
  [3, 'login.live.com', 22, 1, 0x48, 'a port that is not a web port'],
  [5, '127.0.0.1', 443, 1, 0x48, 'a loopback address'],
  [7, '169.254.169.254', 80, 1, 0x48, 'the cloud metadata address'],
  [9, 'login.live.com.evil.com', 443, 1, 0x03, 'a look-alike name, also just a name now'],
  [11, 'login.live.com', 443, 2, 0x48, 'UDP'],
  [13, 'login.live.com', 443, 1, 0x03, 'Microsoft, but with no confirmed Cloudflare exit'],
];
for (const [id, host, port, type] of cases) ws.send(connectPacket(id, host, port, type));
await new Promise((r) => setTimeout(r, 1500));
let bad = 0;
for (const [id, host, port, , want, what] of cases) {
  const got = closes.get(id);
  const ok = got === want;
  console.log(`${ok ? 'ok' : 'FAIL'}: ${what} (${host}:${port}) -> CLOSE 0x${got === undefined ? '??' : got.toString(16)}${ok ? '' : `, wanted 0x${want.toString(16)}`}`);
  if (!ok) bad++;
}
ws.close();
process.exit(bad ? 1 : 0);
