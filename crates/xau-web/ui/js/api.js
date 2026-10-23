// The server's side of the same commands the Android app calls through JNI.
// Arguments are strings ("1"/"0" for booleans): the contract Kotlin has
// always used, so the core needs no second dialect for the web.

export class AuthError extends Error {}

const strArgs = (o = {}) =>
  Object.fromEntries(
    Object.entries(o)
      .filter(([, v]) => v != null)
      .map(([k, v]) => [k, typeof v === 'boolean' ? (v ? '1' : '0') : String(v)]),
  );

async function post(path, body) {
  const r = await fetch(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-xau': '1' },
    body: JSON.stringify(body ?? {}),
    credentials: 'same-origin',
  });
  if (r.status === 401) throw new AuthError('abra o link impresso no terminal do xau-web');
  if (!r.ok) throw new Error(`o servidor respondeu ${r.status}`);
  const j = await r.json();
  if ('err' in j) throw new Error(j.err);
  return j.ok;
}

export const call = (method, args) => post(`/api/call/${method}`, strArgs(args));

async function get(path) {
  const r = await fetch(path, { credentials: 'same-origin' });
  if (r.status === 401) throw new AuthError('abra o link impresso no terminal do xau-web');
  if (!r.ok) throw new Error(`o servidor respondeu ${r.status}`);
  return r.json();
}

export const api = {
  info: () => get('/api/info'),

  // accounts
  restore: () => call('restore'),
  refresh: () => call('refresh'),
  select: (xuid) => call('select', { xuid }),
  signOut: (xuid) => call('sign_out', { xuid }),
  setFrozen: (xuid, frozen) => call('set_frozen', { xuid, frozen }),
  singleActive: () => call('single_active').then((r) => !!r.on),
  setSingleActive: (on) => call('set_single_active', { on }),

  // rewards
  missions: (xuid) => call('missions', { xuid }),
  claim: (xuid) => call('claim', { xuid }),
  claimAll: () => call('claim_all'),
  levelLog: () => call('level_log'),
  publicIp: () => call('public_ip').then((r) => r.ip),

  // offers
  offersAll: () => call('offers_all'),
  offerCodes: (offerId) => call('offer_codes', { offerId }),
  offerHistory: () => call('offer_history'),
  offerCsv: (offerId, title) => call('offer_csv', { offerId, title }),
  claimOffers: (offerId) => call('claim_offers_all', { offerId: offerId || '' }),

  // farm
  farmSummary: () => call('farm_summary'),
  farmTitles: () => call('farm_titles'),
  farmPool: () => call('farm_pool'),
  farmNextPool: () => call('farm_next_pool'),
  farmStart: (xuid) => call('farm_start', { xuid }),
  farmStop: (xuid) => call('farm_stop', { xuid }),

  // store
  storeBrowse: ({ query = '', paid = false, cursor = '', microsoft = false, hideFree = false, min, max, want = 36 }) =>
    call('store_browse', {
      query,
      list: paid ? 'TopPaid' : 'TopTrending',
      cursor,
      microsoft,
      hideFree,
      min,
      max,
      want,
    }),
  storePrewarm: () => call('store_prewarm'),
  storePurchaseUrl: (productId) => call('store_purchase_url', { productId }),

  // the windows only the server can open
  loginStart: () => post('/api/login/start').then((r) => r.id),
  loginStatus: (id) => get(`/api/login/${id}`).then((j) => {
    if ('err' in j) throw new Error(j.err);
    return j.ok;
  }),
  loginCancel: (id) => post(`/api/login/${id}/cancel`),
  // the sign-in whose page runs in this browser (see sjlogin.js)
  loginDirectPage: () => get('/api/login/direct/page').then((j) => {
    if ('err' in j) throw new Error(j.err);
    return j.ok;
  }),
  loginDirectStart: () => post('/api/login/direct/start').then((r) => r.id),
  loginDirectMessage: (id, body) => post(`/api/login/direct/${id}/message`, { body }),
  loginDirectCancel: (id) => post(`/api/login/direct/${id}/cancel`),
  checkout: (xuid, productId) => post('/api/checkout', { xuid, productId }),
};
