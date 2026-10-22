// The web version of the app: shell, routes, and the loops that keep it
// current. Every screen talks to the same core the Android app does.

import { h, icon } from './dom.js';
import { api, AuthError } from './api.js';
import { S, set, onChange } from './state.js';
import { toast } from './ui.js';
import { boot, refresh, pollFarm, updateTitle } from './actions.js';
import { tickFarm } from './views/home.js';

const ROUTES = [
  { re: /^#?\/?$/, nav: 'home', load: () => import('./views/home.js'), params: () => ({}) },
  { re: /^#\/missions\/(\d+)$/, nav: 'home', load: () => import('./views/missions.js'), params: (m) => ({ xuid: m[1] }) },
  { re: /^#\/store(?:\/(\d+))?$/, nav: 'home', load: () => import('./views/store.js'), params: (m) => ({ xuid: m[1] }) },
  { re: /^#\/offers$/, nav: 'offers', load: () => import('./views/offers.js'), params: () => ({ view: 'list' }) },
  { re: /^#\/offers\/history$/, nav: 'offers', load: () => import('./views/offers.js'), params: () => ({ view: 'history' }) },
  { re: /^#\/offers\/([\w-]+)$/, nav: 'offers', load: () => import('./views/offers.js'), params: (m) => ({ view: 'detail', id: m[1] }) },
  { re: /^#\/history$/, nav: 'history', load: () => import('./views/history.js'), params: () => ({}) },
  { re: /^#\/settings$/, nav: 'settings', load: () => import('./views/settings.js'), params: () => ({}) },
];

const NAV = [
  { id: 'home', href: '#/', label: 'Início', icon: 'home' },
  { id: 'offers', href: '#/offers', label: 'Vantagens', icon: 'books' },
  { id: 'history', href: '#/history', label: 'Histórico', icon: 'history' },
  { id: 'settings', href: '#/settings', label: 'Ajustes', icon: 'gear' },
];

let current = null;
let navEl = null;
let viewEl = null;
let token = 0;

function buildShell() {
  navEl = h(
    'nav',
    { class: 'nav', 'aria-label': 'Navegação' },
    h(
      'div',
      { class: 'nav-bar' },
      NAV.map((n) =>
        h('a', { class: 'nav-item', href: n.href, dataset: { nav: n.id } }, h('span', { class: 'nav-pill' }, icon(n.icon, 'nav-ic')), h('span', { class: 'caption-strong' }, n.label)),
      ),
    ),
  );
  viewEl = h('main', { class: 'view', id: 'view' });
  document.getElementById('app').replaceChildren(h('div', { class: 'shell' }, navEl, viewEl));
}

async function navigate() {
  const hash = location.hash || '#/';
  const route = ROUTES.map((r) => ({ r, m: hash.match(r.re) })).find((x) => x.m);
  const mine = ++token;
  current?.destroy?.();
  current = null;
  viewEl.replaceChildren();
  window.scrollTo(0, 0);

  const target = route ?? { r: ROUTES[0], m: ['#/'] };
  navEl.querySelectorAll('.nav-item').forEach((a) => a.classList.toggle('on', a.dataset.nav === target.r.nav));
  document.documentElement.classList.toggle('sub', target.r !== ROUTES[0]);
  try {
    const mod = await target.r.load();
    if (mine !== token) return; // another navigation won the race
    current = mod.mount(viewEl, target.r.params(target.m)) ?? null;
  } catch (e) {
    if (mine === token) viewEl.append(h('div', { class: 'state' }, h('h3', { class: 'danger' }, 'Não foi possível abrir esta tela'), h('p', null, e.message)));
  }
}

function authScreen() {
  document.getElementById('app').replaceChildren(
    h(
      'div',
      { class: 'welcome' },
      h('div', { class: 'logo big' }, icon('xbox')),
      h('h1', null, 'Xbox ', h('span', { class: 'green' }, 'Rewards')),
      h('p', { class: 't2', style: { marginTop: '24px' } }, 'Abra o link que o xau-web mostrou no terminal: ele traz a chave deste computador e só precisa ser aberto uma vez.'),
    ),
  );
}

async function main() {
  // Dev only: canned account data, for looking at every screen without a login.
  if (new URLSearchParams(location.search).has('mock')) await import('./mock.js');

  window.addEventListener('unhandledrejection', (e) => {
    if (e.reason instanceof AuthError) return authScreen();
    toast(e.reason?.message || 'Algo deu errado', 'err');
  });

  try {
    S.info = await api.info();
  } catch (e) {
    if (e instanceof AuthError) return authScreen();
    document.getElementById('app').replaceChildren(h('div', { class: 'state' }, h('h3', { class: 'danger' }, 'O servidor não respondeu'), h('p', null, e.message)));
    return;
  }

  buildShell();
  onChange(() => {
    current?.update?.();
    updateTitle();
  });
  window.addEventListener('hashchange', navigate);
  await navigate();

  boot();

  // The farm's clock counts every second from the last real number.
  setInterval(tickFarm, 1000);
  // The farm's progress, from the server's memory: no request to Xbox.
  setInterval(pollFarm, 5000);
  // Rewards, once a minute, like the app.
  setInterval(() => document.visibilityState === 'visible' && S.accounts.length && refresh({ quiet: true }), 60_000);
  let lastSeen = Date.now();
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && Date.now() - lastSeen > 30_000 && S.accounts.length) refresh({ quiet: true });
    lastSeen = Date.now();
  });
  set({});
}

main();
