// DOM and formatting helpers. Text is always inserted as text nodes, never as
// HTML: nothing the Store, Xbox or an account returns can inject markup. The
// only innerHTML is the app's own icon constants.

import { ICONS } from './icons.js';

export function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
      else if (k === 'dataset') Object.assign(el.dataset, v);
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2).toLowerCase(), v);
      else if (v === true) el.setAttribute(k, '');
      else el.setAttribute(k, v);
    }
  }
  append(el, kids);
  return el;
}

/** Replace the children of el. Unlike replaceChildren, false/null are skipped, not written as text. */
export function put(el, ...kids) {
  el.replaceChildren();
  return append(el, kids);
}

export function append(el, kids) {
  for (const k of kids.flat(Infinity)) {
    if (k == null || k === false) continue;
    el.append(k instanceof Node ? k : document.createTextNode(String(k)));
  }
  return el;
}

/** One of the app's own icons, drawn in the current text colour. */
export function icon(name, cls = '') {
  const s = document.createElement('span');
  s.className = `ic ${cls}`.trim();
  s.innerHTML = ICONS[name] || '';
  return s;
}

/** An image from the Store or Xbox: no referrer, lazy, and nothing if it fails. */
export function img(url, alt = '') {
  if (!url) return null;
  const el = h('img', { src: url, alt, loading: 'lazy', decoding: 'async', referrerpolicy: 'no-referrer' });
  el.addEventListener('error', () => el.remove());
  return el;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function debounce(fn, ms) {
  let t;
  return (...a) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}

// --------------------------------------------------------------- format

const nf = new Intl.NumberFormat('pt-BR');

/** 4522 -> "4.522"; null -> an em dash, never a made-up 0. */
export const money = (n) => (n == null ? '—' : nf.format(Math.round(Number(n))));

export const plural = (n, one, many) => (n === 1 ? one : many);

/** 2018-08-02 -> 02/08/2018 */
export function brDate(iso) {
  const p = String(iso).split('-');
  return p.length === 3 ? `${p[2]}/${p[1]}/${p[0]}` : String(iso);
}

/** An ISO instant as `28/07/2026 · 15:24` in this computer's zone. */
export function prettyDate(iso) {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getDate())}/${p(d.getMonth() + 1)}/${d.getFullYear()} · ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function timeOf(unix) {
  const d = new Date(unix * 1000);
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function dayOf(unix) {
  const d = new Date(unix * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

export function dayLabel(key) {
  const today = dayOf(Date.now() / 1000);
  const yesterday = dayOf(Date.now() / 1000 - 86400);
  if (key === today) return 'HOJE';
  if (key === yesterday) return 'ONTEM';
  return brDate(key);
}

/** Seconds as m:ss. */
export function clock(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** "9d restantes" until an ISO reset, rounded up like the official page. */
export function daysLeft(iso) {
  const end = iso ? new Date(iso).getTime() : NaN;
  if (Number.isNaN(end)) return null;
  const secs = (end - Date.now()) / 1000;
  if (secs <= 0) return null;
  const hours = Math.ceil(secs / 3600);
  return hours < 24 ? `${hours}h restantes` : `${Math.ceil(hours / 24)}d restantes`;
}

/** The service answers in English; the app speaks Portuguese. */
export const ptBr = (s) => String(s ?? '').replace(/^Day (\d+)$/, 'Dia $1').replace('Next week', 'Próx. semana');

export function prettyTier(tier) {
  const t = String(tier).toLowerCase();
  if (t.includes('ultimate')) return 'Game Pass Ultimate';
  if (t.includes('pc')) return 'PC Game Pass';
  if (t.includes('console')) return 'Game Pass Console';
  if (t.includes('core')) return 'Game Pass Core';
  if (t.includes('standard')) return 'Game Pass Standard';
  return 'Game Pass';
}

export function prettyType(t) {
  if (t == null) return 'Vantagem';
  if (/external-link/i.test(t)) return 'Link externo';
  if (/store-token/i.test(t)) return 'Código da Store';
  return t;
}

/** A medal before a Rewards tier's name: silver for Silver, gold for Gold. */
export function medal(tier) {
  if (/gold/i.test(tier)) return '🥇 ';
  if (/silver/i.test(tier)) return '🥈 ';
  return '';
}

export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Clipboard access needs a secure context; localhost is one, a LAN address is not.
    const ta = h('textarea', { style: { position: 'fixed', opacity: '0' } });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand('copy');
    } catch {
      ok = false;
    }
    ta.remove();
    return ok;
  }
}

/** Animate a number up to `target` (the app's count-up). */
export function countUp(el, target, formatter = money) {
  if (target == null) {
    el.textContent = '—';
    el._v = null;
    return;
  }
  const from = el._v ?? 0;
  el._v = target;
  if (from === target || matchMedia('(prefers-reduced-motion: reduce)').matches) {
    el.textContent = formatter(target);
    return;
  }
  const t0 = performance.now();
  const dur = 900;
  const step = (now) => {
    const k = Math.min(1, (now - t0) / dur);
    const eased = 1 - Math.pow(1 - k, 3);
    el.textContent = formatter(from + (target - from) * eased);
    if (k < 1 && el._v === target) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}
