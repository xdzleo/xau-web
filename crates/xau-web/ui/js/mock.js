// Development only: `/?mock` answers the account commands with made-up data
// so every screen can be looked at without signing in. Store commands still
// go to the real server. Nothing here runs unless the URL asks for it, and
// none of it touches a real account.

const real = window.fetch.bind(window);
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

/** A flat tile as a picture: stands in for cover art and avatars. */
function art(label, hue = 120, w = 200, h = 200) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue},55%,38%)"/><stop offset="1" stop-color="hsl(${(hue + 50) % 360},60%,16%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="50%" y="54%" text-anchor="middle" font-family="sans-serif" font-weight="700" font-size="${Math.round(h / 3)}" fill="rgba(255,255,255,.85)">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

const T0 = Math.floor(Date.now() / 1000); // fixed: a log whose dates move would re-announce itself
const now = () => T0;

let accounts = [
  { xuid: '1001', gamertag: 'AuroraPlays', gamerpic: art('A', 150, 96, 96), gamerscore: 38127, farming: true, active: true, frozen: false, staleReason: null, email: 'aurora@example.com', farmPercent: 33, farmCycle: 1, farmElapsed: 300, farmTotal: 900, farmState: 'Farming', farmPresence: 'accepted', balance: 4522, unclaimed: 40, level: 2, lifetime: 25322, missionsOpen: 5, missionsDone: 3, gamepass: 'GamePassUltimate', tierProgress: { tier: 'Membro Silver', next: 'Membro Gold', progress: 736, max: 750, requirement: 'Ganhe pelo menos 750 pontos' }, memberSince: '2018-08-02' },
  { xuid: '1002', gamertag: 'BrunoX', gamerpic: art('B', 30, 96, 96), gamerscore: 12044, farming: false, active: false, frozen: false, staleReason: null, email: 'bruno@example.com', balance: 1210, unclaimed: 0, level: 1, lifetime: 9820, missionsOpen: 7, missionsDone: 1, gamepass: null, tierProgress: { tier: 'Membro', next: 'Membro Silver', progress: 120, max: 750, requirement: null }, memberSince: '2020-03-11' },
  { xuid: '1003', gamertag: 'Casa11', gamerpic: art('C', 270, 96, 96), gamerscore: 880, farming: false, active: false, frozen: true, staleReason: null, email: 'casa@example.com', balance: 2097, unclaimed: 0, level: 2, lifetime: 155413, memberSince: '2015-02-10', cached: true },
  { xuid: '1004', gamertag: 'DaviOld', gamerpic: art('D', 200, 96, 96), gamerscore: 5000, farming: false, active: false, frozen: false, staleReason: 'sessão expirada: HTTP status 401', email: null, balance: null, unclaimed: null, level: null },
];
let singleActive = false;
let pool = 1;
let farmStartedAt = Date.now() - 300_000;

const TITLES = ['Doom Eternal', 'CoD Black Ops', 'Halo Infinite', 'Forza Horizon 5', 'Gears 5', 'Starfield', 'Forza Motorsport', 'Halo MCC'];
const titles = () => TITLES.map((n, i) => ({ titleId: String(100 + i), name: n, image: art(n.slice(0, 2).toUpperCase(), (i * 43 + pool * 20) % 360) }));

const rows = () => accounts.map((a) => ({ ...a }));

const missions = [
  { id: 'mobile', title: 'Use o app XBOX mobile', points: 0, label: 'Available', progress: 0, done: false, open: true, description: 'Entre no app XBOX mobile.', kind: 'streak', steps: [{ label: 'Day 1', points: 2, done: true, note: null }, { label: 'Day 2', points: 6, done: true, note: null }, { label: 'Day 3', points: 10, done: false, note: null }, { label: 'Day 4', points: 14, done: false, note: null }, { label: 'Day 5', points: 18, done: false, note: null }], current: 2, count: 2, target: 5, claimed: 6, locked: false, requirement: null, failed: false, thumbnail: null, resetDate: new Date(Date.now() + 9 * 86400e3).toISOString() },
  { id: 'console', title: 'Bônus semanal do console', points: 110, label: 'Available', progress: 0, done: false, open: true, description: 'Jogue 5 dias nesta semana.', kind: 'streak', steps: [{ label: 'Day 1', points: 0, done: false, note: null }, { label: 'Day 2', points: 0, done: false, note: null }, { label: 'Day 3', points: 0, done: false, note: null }, { label: 'Day 4', points: 0, done: false, note: null }, { label: 'Day 5', points: 110, done: false, note: null }], current: 0, count: 0, target: 5, claimed: null, locked: false, requirement: null, failed: true, thumbnail: null, resetDate: new Date(Date.now() + 3 * 86400e3).toISOString() },
  { id: 'gp-weekly', title: 'Sequência semanal do Game Pass', points: 0, label: 'Locked', progress: 0, done: false, open: false, description: 'Jogue um jogo do Game Pass por 5 dias.', kind: 'streak', steps: [{ label: '1d', points: 0, done: false, note: null }, { label: '2d', points: 0, done: false, note: null }, { label: '3d', points: 0, done: false, note: null }, { label: '4d', points: 0, done: false, note: null }, { label: '5d', points: 40, done: false, note: 'Next week 2x' }, { label: '6d', points: 10, done: false, note: null }, { label: '7d', points: 20, done: false, note: null }], current: null, count: 0, target: 7, claimed: null, locked: true, requirement: null, failed: false, thumbnail: null, resetDate: null },
  { id: 'pack4', title: 'Pacote mensal de 4 do Game Pass', points: 40, label: 'Locked', progress: 0, done: false, open: false, description: 'Pontue jogando quatro jogos do Game Pass todo mês.', kind: 'pack', steps: TITLES.slice(0, 4).map((n, i) => ({ label: '', points: 0, done: true, note: null, image: art(n.slice(0, 2).toUpperCase(), i * 60) })), current: null, count: 4, target: 4, claimed: null, locked: true, requirement: 'Requer assinatura Game Pass', failed: false, thumbnail: null, resetDate: new Date(Date.now() + 20 * 86400e3).toISOString() },
  { id: 'pack8', title: 'Pacote mensal de 8 do Game Pass', points: 280, label: 'Available', progress: 50, done: false, open: true, description: 'Jogue oito jogos do Game Pass por mês.', kind: 'pack', steps: TITLES.map((n, i) => ({ label: '', points: 0, done: i < 4, note: null, image: i < 4 ? art(n.slice(0, 2).toUpperCase(), i * 60) : null })), current: 4, count: 4, target: 8, claimed: null, locked: false, requirement: null, failed: false, thumbnail: null, resetDate: new Date(Date.now() + 20 * 86400e3).toISOString() },
  { id: 'single1', title: 'Jogue 8 jogos por 15 minutos', points: 10, label: 'Available', progress: 40, done: false, open: true, description: 'Fique em presença em oito jogos.', kind: 'single', steps: [], current: null, count: null, target: null, claimed: null, locked: false, requirement: null, failed: false, thumbnail: art('8', 120, 112, 112), resetDate: new Date(Date.now() + 14 * 3600e3).toISOString() },
  { id: 'single2', title: 'Faça login no Xbox', points: 5, label: 'Claimed', progress: 100, done: true, open: false, description: '', kind: 'single', steps: [], current: null, count: null, target: null, claimed: 5, locked: false, requirement: null, failed: false, thumbnail: null, resetDate: null },
];

let offerClaimed = false;
const offersFor = (a) => [
  { offerId: 'nitro', title: 'Discord Nitro', description: '1 mês de Nitro', status: offerClaimed ? 'Claimed' : 'Available', resourceType: 'external-link', resource: offerClaimed ? 'https://discord.com/billing/promotions/ABCD1234' : null, claimedDate: offerClaimed ? new Date().toISOString() : null, available: !offerClaimed, claimed: offerClaimed },
  { offerId: 'store', title: 'Código da Microsoft Store', description: null, status: 'Claimed', resourceType: 'store-token', resource: `PAYXX-${a.xuid}-ZZZ99`, claimedDate: '2026-09-02T18:24:59Z', available: false, claimed: true },
  { offerId: 'old', title: null, description: null, status: 'Expired', resourceType: 'store-token', resource: null, claimedDate: null, available: false, claimed: false },
];

const handlers = {
  restore: async () => rows(),
  refresh: async () => rows(),
  select: async ({ xuid }) => { accounts.forEach((a) => (a.active = a.xuid === xuid)); return { ok: true }; },
  sign_out: async ({ xuid }) => { accounts = accounts.filter((a) => a.xuid !== xuid); return rows(); },
  set_frozen: async ({ xuid, frozen }) => {
    const a = accounts.find((x) => x.xuid === xuid);
    a.frozen = frozen === '1';
    if (a.frozen) { a.farming = false; a.cached = true; } else { a.cached = false; }
    if (singleActive && !a.frozen) accounts.forEach((o) => o.xuid !== xuid && (o.frozen = true, o.farming = false, o.cached = true));
    return rows();
  },
  single_active: async () => ({ on: singleActive }),
  set_single_active: async ({ on }) => {
    singleActive = on === '1';
    if (singleActive) { const keep = accounts.find((a) => a.active && !a.frozen) || accounts.find((a) => !a.frozen); accounts.forEach((a) => a !== keep && (a.frozen = true, a.farming = false, a.cached = true)); }
    return rows();
  },
  missions: async () => missions,
  level_log: async () => [
    { xuid: '1001', gamertag: 'AuroraPlays', gamerpic: accounts[0].gamerpic, from: 1, to: 2, at: now() - 3600, peak: 4522 },
    { xuid: '1003', gamertag: 'Casa11', gamerpic: accounts[2].gamerpic, from: 2, to: 3, at: now() - 86400 - 7200, peak: 155413 },
    { xuid: '1002', gamertag: 'BrunoX', gamerpic: accounts[1].gamerpic, from: 1, to: 2, at: now() - 5 * 86400, peak: 1210 },
  ],
  claim: async ({ xuid }) => { const a = accounts.find((x) => x.xuid === xuid); const added = a.unclaimed || 0; a.balance += added; a.unclaimed = 0; return { pointsAdded: added, balance: a.balance, accounts: rows() }; },
  claim_all: async () => { let n = 0; accounts.forEach((a) => { if (a.balance == null) return; n += a.unclaimed || 0; a.balance += a.unclaimed || 0; a.unclaimed = 0; }); return { pointsAdded: n, gamertags: [], accounts: rows() }; },
  public_ip: async () => ({ ip: '203.0.113.7' }),
  farm_titles: async () => titles(),
  farm_pool: async () => ({ pool, pools: 3 }),
  farm_next_pool: async () => { pool = (pool % 3) + 1; return { pool, pools: 3 }; },
  farm_start: async ({ xuid }) => { const a = accounts.find((x) => x.xuid === xuid); if (a.frozen) throw new Error('conta congelada: descongele para usar'); a.farming = true; a.farmElapsed = 0; farmStartedAt = Date.now(); return { running: true }; },
  farm_stop: async ({ xuid }) => { accounts.find((x) => x.xuid === xuid).farming = false; return { running: false }; },
  farm_summary: async () => accounts.filter((a) => a.farming).map((a) => ({ xuid: a.xuid, gamertag: a.gamertag, running: true, elapsed: Math.min(900, Math.floor((Date.now() - farmStartedAt) / 1000) + 300), total: 900, presence: 'accepted' })),
  offers_all: async () => accounts.filter((a) => !a.frozen && !a.staleReason).map((a) => ({ xuid: a.xuid, gamertag: a.gamertag, error: null, offers: offersFor(a) })).concat([{ xuid: '1004', gamertag: 'DaviOld', error: 'sessão expirada', offers: [] }]),
  offer_codes: async ({ offerId }) => (offerId === 'nitro' && !offerClaimed ? [] : [{ gamertag: 'AuroraPlays', code: 'https://discord.com/billing/promotions/ABCD1234', claimedDate: '2026-09-02T18:24:59Z' }, { gamertag: 'BrunoX', code: 'PAYXX-1002-ZZZ99', claimedDate: '2026-08-02T10:00:00Z' }]),
  offer_history: async () => [
    { title: 'Discord Nitro', resourceType: 'external-link', codes: [{ gamertag: 'AuroraPlays', offerId: 'nitro', code: 'https://discord.com/billing/promotions/ABCD1234', claimedDate: '2026-09-02T18:24:59Z' }] },
    { title: '', resourceType: 'store-token', codes: [{ gamertag: 'BrunoX', offerId: 'store', code: 'PAYXX-1002-ZZZ99', claimedDate: '2026-08-02T10:00:00Z' }] },
  ],
  offer_csv: async ({ offerId, title }) => ({ fileName: `${(title || offerId).replace(/\W+/g, '_')}.csv`, text: 'gamertag,code,claimed\r\nAuroraPlays,ABCD,2026-09-02\r\n' }),
  claim_offers_all: async () => { offerClaimed = true; return { claimed: [{ gamertag: 'AuroraPlays', offerId: 'nitro', title: 'Discord Nitro', resourceType: 'external-link', resource: 'https://discord.com/x', csv: 'C:\\data\\nitro.csv' }], failures: [], indexCsv: 'C:\\data\\index.csv' }; },
};

let loginSince = 0;

window.fetch = async (url, opts) => {
  const u = typeof url === 'string' ? url : url.url;
  const m = u.match(/^\/api\/call\/(\w+)/);
  if (m && handlers[m[1]]) {
    await delay(120);
    try {
      const args = JSON.parse(opts?.body || '{}');
      return json({ ok: await handlers[m[1]](args) });
    } catch (e) {
      return json({ err: e.message });
    }
  }
  if (u === '/api/login/start') { loginSince = Date.now(); return json({ ok: { id: 'mock' } }); }
  if (u === '/api/login/mock') {
    const t = Date.now() - loginSince;
    if (t < 3000) return json({ ok: { state: 'waiting' } });
    if (t < 5000) return json({ ok: { state: 'working' } });
    return json({ ok: { state: 'done', accounts: rows(), warning: null } });
  }
  if (u.startsWith('/api/login/') && u.endsWith('/cancel')) return json({ ok: {} });
  if (u === '/api/checkout') return json({ ok: {} });
  return real(url, opts);
};

console.info('[mock] account commands are canned; the Store is real');
