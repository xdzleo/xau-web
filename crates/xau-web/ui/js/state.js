// What the screens share: the accounts as the core last reported them, the
// farm's games, and the user's browser-side preferences.

export const S = {
  booting: true,
  /** The core's account rows, as the app's `Account`. */
  accounts: [],
  /** A failure that stopped the whole screen, not one row. */
  fatal: null,
  refreshing: false,
  claimingXuid: null,
  farmTitles: [],
  farmPool: 1,
  farmPools: 3,
  singleActive: false,
  info: null,
  publicIp: null,
};

const listeners = new Set();
let queued = false;

export function onChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Merge into the state and tell the screens, once per tick. */
export function set(patch) {
  Object.assign(S, patch);
  if (queued) return;
  queued = true;
  queueMicrotask(() => {
    queued = false;
    for (const fn of listeners) fn();
  });
}

/** Frozen accounts never count: nothing is asked for them. */
export const live = () => S.accounts.filter((a) => !a.frozen);
export const totalWaiting = () => live().reduce((n, a) => n + (a.unclaimed || 0), 0);
export const totalBalance = () => {
  const l = live();
  return l.some((a) => a.balance == null) ? null : l.reduce((n, a) => n + (a.balance || 0), 0);
};
export const active = () => live().find((a) => a.active) || live()[0];
export const byXuid = (xuid) => S.accounts.find((a) => a.xuid === xuid);

// ----------------------------------------------------------- preferences

export const prefs = {
  get(key, fallback) {
    try {
      const v = localStorage.getItem(`xau.${key}`);
      return v == null ? fallback : JSON.parse(v);
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(`xau.${key}`, JSON.stringify(value));
    } catch {
      /* private mode: the choice lasts until the tab closes */
    }
  },
};
