// What the user can do to an account, and what keeps the screens current.
// The same behaviour as the Android `AppViewModel`: same refresh cadence,
// same auto-claim backoff, same notices.

import { api } from './api.js';
import { S, set, byXuid, prefs, totalWaiting } from './state.js';
import { toast, confirmDialog } from './ui.js';
import { notify } from './notify.js';

const AUTO_BACKOFF_MS = 30 * 60_000;
let autoBlockedUntil = 0;
let autoClaiming = false;

/** Anchor each row's farm clock to now, so the screen can count from it. */
function stamp(rows) {
  const now = performance.now();
  for (const r of rows) r.farmAt = now;
  return rows;
}

export function updateTitle() {
  const n = S.accounts.filter((a) => a.farming).length;
  document.title = n ? `● Farmando ${n}/${S.accounts.length} · Xbox Rewards` : 'Xbox Rewards';
}

/** Rebuild every stored session and read Rewards once, at start. */
export async function boot() {
  try {
    set({ accounts: stamp(await api.restore()), booting: false });
  } catch (e) {
    set({ booting: false, fatal: e.message });
    return;
  }
  if (S.accounts.length) await refresh();
  api.singleActive().then((on) => set({ singleActive: on })).catch(() => {});
  // The store then opens from the native cache instead of the network.
  api.storePrewarm().catch(() => {});
  api.publicIp().then((ip) => set({ publicIp: ip })).catch(() => {});
}

export async function refresh({ quiet = false } = {}) {
  if (!quiet) set({ refreshing: true });
  try {
    const rows = stamp(await api.refresh());
    set({ accounts: rows, refreshing: false, fatal: null });
    loadFarmTitles();
    checkLevelUps();
    maybeAutoClaim();
  } catch (e) {
    // A failed refresh must not leave stale figures looking current: the
    // rows the core returns already carry null balances. Only say so when
    // the user asked for the refresh.
    set({ refreshing: false, fatal: quiet ? S.fatal : e.message });
  }
}

export async function select(xuid) {
  try {
    await api.select(xuid);
    set({ accounts: S.accounts.map((a) => ({ ...a, active: a.xuid === xuid })) });
  } catch {
    /* an account that is gone: the next refresh drops it */
  }
}

export async function signOut(xuid) {
  const a = byXuid(xuid);
  const ok = await confirmDialog({
    title: `Remover ${a?.gamertag || 'a conta'}?`,
    text: 'A sessão sai deste computador e o farm desta conta para. Os pontos já resgatados não são afetados.',
    confirm: 'Remover',
    danger: true,
  });
  if (!ok) return;
  try {
    set({ accounts: stamp(await api.signOut(xuid)) });
    toast('Conta removida');
  } catch (e) {
    toast(e.message, 'err');
  }
}

// ------------------------------------------------------------- rewards

export async function claim(xuid) {
  set({ claimingXuid: xuid });
  try {
    const out = await api.claim(xuid);
    set({ accounts: stamp(out.accounts), claimingXuid: null });
    toast(out.pointsAdded > 0 ? `+${out.pointsAdded} pontos resgatados` : 'Nada foi creditado');
  } catch (e) {
    set({ claimingXuid: null });
    toast(e.message, 'err');
  }
}

/**
 * Claim wherever points are waiting, without a tap. One pass at a time, and
 * a 30-minute pause after a sweep that credits nothing or fails: both mean
 * the service said no, and asking again a minute later would only look
 * like abuse.
 */
async function maybeAutoClaim() {
  if (autoClaiming || Date.now() < autoBlockedUntil || totalWaiting() <= 0) return;
  autoClaiming = true;
  try {
    const out = await api.claimAll();
    set({ accounts: stamp(out.accounts) });
    if (out.pointsAdded > 0) {
      toast(`+${out.pointsAdded} resgatados automaticamente`);
    } else {
      autoBlockedUntil = Date.now() + AUTO_BACKOFF_MS;
      toast('Resgate automático: nada creditado — pausado 30 min');
    }
  } catch {
    autoBlockedUntil = Date.now() + AUTO_BACKOFF_MS;
    toast('Resgate automático falhou — pausado 30 min', 'err');
  } finally {
    autoClaiming = false;
  }
}

// ------------------------------------------------------------ accounts

/** Freeze an account (it goes silent, its farm stops) or bring it back. */
export async function toggleFrozen(xuid) {
  const freeze = byXuid(xuid)?.frozen !== true;
  try {
    set({ accounts: stamp(await api.setFrozen(xuid, freeze)) });
    updateTitle();
    toast(freeze ? 'Conta congelada' : 'Conta descongelada');
  } catch (e) {
    toast(e.message, 'err');
  }
}

/** "Uma conta ativa por vez": on freezes every account but one. */
export async function setSingleActive(on) {
  try {
    const rows = stamp(await api.setSingleActive(on));
    set({ accounts: rows, singleActive: on });
    toast(on ? 'Só uma conta ativa por vez' : 'Várias contas ativas liberadas');
  } catch (e) {
    toast(e.message, 'err');
  }
}

// ---------------------------------------------------------------- farm

export async function toggleFarm(xuid) {
  const running = byXuid(xuid)?.farming === true;
  try {
    await (running ? api.farmStop(xuid) : api.farmStart(xuid));
    await refresh({ quiet: true });
  } catch (e) {
    toast(e.message, 'err');
  }
}

/** The farm's games with their box art; asked again until the art arrived. */
export async function loadFarmTitles(force = false) {
  if (!force && S.farmTitles.some((t) => t.image)) return;
  try {
    set({ farmTitles: await api.farmTitles() });
  } catch {
    /* decoration: the card falls back to initials */
  }
  try {
    const { pool, pools } = await api.farmPool();
    set({ farmPool: pool, farmPools: pools });
  } catch {
    /* keeps the last */
  }
}

/** "Trocar jogos": the next pool of eight Microsoft games. */
export async function nextPool() {
  try {
    const { pool, pools } = await api.farmNextPool();
    set({ farmPool: pool, farmPools: pools });
    toast(S.accounts.some((a) => a.farming) ? `Pool ${pool} de ${pools}: vale no próximo farm` : `Pool ${pool} de ${pools}`);
    await loadFarmTitles(true);
  } catch (e) {
    toast(e.message, 'err');
  }
}

/**
 * The farm's progress, from memory on the server (no request to Xbox): which
 * farms are running, how far, and whether Xbox accepts the pings. A farm that
 * just ended its 15 minutes says so.
 */
export async function pollFarm() {
  if (!S.accounts.some((a) => a.farming)) return;
  let rows;
  try {
    rows = await api.farmSummary();
  } catch {
    return;
  }
  const now = performance.now();
  let ended = false;
  let presenceChanged = false;
  for (const a of S.accounts) {
    const r = rows.find((x) => x.xuid === a.xuid);
    const running = !!r?.running;
    if (r) {
      if (a.farmPresence !== r.presence) presenceChanged = true;
      a.farmElapsed = r.elapsed;
      a.farmTotal = r.total;
      a.farmPresence = r.presence;
      a.farmAt = now;
    }
    if (a.farming && !running) {
      ended = true;
      if (r && r.total > 0 && r.elapsed >= r.total - 5) {
        notify('done', `${a.gamertag} terminou o farm`, 'Completou os 15 minutos nos 8 jogos.', `done-${a.xuid}`);
      }
    }
    a.farming = running;
  }
  if (ended || presenceChanged) {
    set({ accounts: S.accounts });
    if (ended) refresh({ quiet: true });
  }
}

// ----------------------------------------------------------- level-ups

/**
 * Every live read the server makes logs a level-up; this tells the user
 * about the ones that are new. The first run only learns what is already
 * there, so an old log does not fire a pile of notices.
 */
export async function checkLevelUps() {
  let log;
  try {
    log = await api.levelLog();
  } catch {
    return;
  }
  const newest = log.reduce((m, e) => Math.max(m, e.at), 0);
  const seen = prefs.get('levelSeenAt', null);
  if (seen == null) {
    prefs.set('levelSeenAt', newest);
    return;
  }
  const fresh = log.filter((e) => e.at > seen);
  if (!fresh.length) return;
  prefs.set('levelSeenAt', newest);
  for (const e of fresh.reverse()) {
    notify('level', `${e.gamertag} subiu para o nível ${e.to}`, `Nível ${e.from} → ${e.to} no Microsoft Rewards.`, `level-${e.xuid}-${e.to}`);
  }
}
