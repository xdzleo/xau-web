// Início: the total, the farm's games, and one card per account.

import { h, icon, countUp, money, brDate, clock, copyText, medal, prettyTier } from '../dom.js';
import { S, byXuid, live, totalBalance, totalWaiting } from '../state.js';
import { toast, skeleton, gameArt, avatar, switchEl } from '../ui.js';
import * as act from '../actions.js';
import { startLogin } from './login.js';

let lastTotal = null; // so the total counts from what it showed, not from zero
const lastBalance = new Map();

export function mount(root) {
  const page = h('div', { class: 'page' });
  root.append(page);

  function render(enter) {
    const next = h('div', null);
    if (!S.booting && S.accounts.length === 0) {
      next.append(welcome(enter));
    } else {
      next.append(...sections(enter));
    }
    page.replaceChildren(...next.childNodes);
  }
  render(true);
  return {
    update: () => render(false),
    destroy: () => {},

  };
}

function wrapRise(enter, i, el) {
  if (enter) {
    el.classList.add('rise');
    el.style.setProperty('--i', i);
  }
  return el;
}

function sections(enter) {
  const out = [];
  out.push(wrapRise(enter, 0, topBar()));
  if (S.fatal) {
    out.push(
      h(
        'div',
        { class: 'rowcard', style: { marginTop: '16px', borderColor: 'var(--danger)' } },
        h('div', { class: 'danger' }, S.fatal),
      ),
    );
  }
  out.push(wrapRise(enter, 1, hero()));
  if (S.farmTitles.length) out.push(wrapRise(enter, 2, farmGames()));
  out.push(wrapRise(enter, 3, accountsHead()));
  if (S.booting) {
    out.push(h('div', { class: 'accounts' }, skeleton(340, 280), skeleton(340, 280)));
  } else {
    const grid = h('div', { class: 'accounts' });
    S.accounts.forEach((a, i) => grid.append(wrapRise(enter, 4 + i, a.frozen ? frozenCard(a) : accountCard(a))));
    out.push(grid);
  }
  // The page's own rhythm: every block 16px apart.
  out.forEach((el, i) => i > 0 && (el.style.marginTop ||= '16px'));
  return out;
}

// ------------------------------------------------------------- top bar

function topBar() {
  return h(
    'div',
    { class: 'topbar' },
    h('div', { class: 'logo' }, icon('xbox')),
    h(
      'div',
      { class: 'grow' },
      h('div', { class: 'brand' }, 'Xbox ', h('span', { class: 'green' }, 'Rewards')),
      h('div', { class: 'caption t2' }, 'Jogos. Presença. Recompensas.'),
    ),
    h(
      'button',
      {
        class: `iconbtn${S.refreshing ? ' spin' : ''}`,
        'aria-label': 'Atualizar',
        title: 'Atualizar',
        onClick: () => act.refresh(),
      },
      icon('refresh'),
    ),
  );
}

// ---------------------------------------------------------------- hero

function ring(done, total) {
  const C = 2 * Math.PI * 45;
  const f = total ? done / total : 0;
  const el = h('div', { class: 'ring' });
  el.innerHTML =
    `<svg viewBox="0 0 104 104" aria-hidden="true"><defs><linearGradient id="rg" x1="0" x2="1"><stop offset="0" stop-color="#107c10"/><stop offset=".5" stop-color="#3fd44b"/><stop offset="1" stop-color="#107c10"/></linearGradient></defs>` +
    `<circle cx="52" cy="52" r="45" fill="none" stroke="rgba(0,0,0,.35)" stroke-width="10"/>` +
    `<circle cx="52" cy="52" r="45" fill="none" stroke="url(#rg)" stroke-width="10" stroke-linecap="round" stroke-dasharray="${(f * C).toFixed(1)} ${C.toFixed(1)}"/></svg>`;
  el.append(h('div', { class: 'ctr' }, h('div', { class: 'ring-fig' }, `${done}/${total}`), h('div', { class: 'caption t2' }, 'no farm')));
  return el;
}

function stat(label, value, { live: isLive = false, accent = false } = {}) {
  return h(
    'div',
    { class: 'stat' },
    h('div', { class: 'label' }, isLive && h('i', { class: 'livedot' }), label),
    h('div', { class: `value${accent ? ' accent' : ''} ellip` }, value),
  );
}

function hero() {
  const total = S.accounts.length;
  const farming = S.accounts.filter((a) => a.farming).length;
  const withGp = S.accounts.filter((a) => a.gamepass != null).length;
  const waiting = totalWaiting();

  const fig = h('div', { class: 'hero-fig ellip' });
  if (S.booting) fig.textContent = '—';
  else {
    fig._v = lastTotal;
    const t = totalBalance();
    countUp(fig, t);
    lastTotal = t;
  }
  return h(
    'div',
    { class: 'hero' },
    h(
      'div',
      { class: 'hero-top' },
      h(
        'div',
        { class: 'grow' },
        h('div', { class: 'eyebrow' }, 'SALDO TOTAL'),
        fig,
        h('div', { class: 'body t2' }, total === 0 ? 'nenhuma conta conectada' : total === 1 ? 'pontos em 1 conta' : `pontos em ${total} contas`),
      ),
      ring(farming, total),
    ),
    h(
      'div',
      { class: 'stats' },
      stat('Farmando', `${farming}/${total}`, { live: farming > 0 }),
      stat('Game Pass', `${withGp}/${total}`),
      stat('A resgatar', money(waiting), { accent: waiting > 0 }),
    ),
  );
}

// ---------------------------------------------------------- farm games

function farmGames() {
  const farming = S.accounts.some((a) => a.farming);
  const titles = S.farmTitles;
  return h(
    'div',
    { class: 'card farmgames' },
    h(
      'div',
      { class: 'head' },
      icon('pad'),
      h(
        'div',
        { class: 'grow' },
        h('div', { class: 'section' }, 'Jogos do farm'),
        h('div', { class: 'caption t2' }, `${titles.length} jogos Microsoft · pool ${S.farmPool} de ${S.farmPools} · 15 min`),
      ),
      h('button', { class: 'smallbtn', title: 'Trocar jogos (próximo pool)', 'aria-label': 'Trocar jogos', onClick: () => act.nextPool() }, '⟳'),
      h('span', { class: `pill${farming ? ' ok' : ''}` }, h('i', { class: `livedot sm${farming ? '' : ' off'}` }), farming ? 'Online' : 'Parado'),
    ),
    h(
      'div',
      { class: `titles${farming ? '' : ' dim'}` },
      titles.map((t) => h('div', { class: 'titlecell' }, gameArt(t), h('div', { class: 'name ellip', title: t.name }, t.name))),
    ),
  );
}

// ------------------------------------------------------------ accounts

function accountsHead() {
  return h(
    'div',
    { class: 'sectionhead' },
    h('div', { class: 'section' }, 'Contas'),
    h('span', { class: 'countbadge' }, S.accounts.length),
    h('button', { class: 'addbtn', onClick: () => startLogin() }, '+ Adicionar'),
  );
}

function levelBadge(level, stale) {
  const tier = stale || level == null ? null : level >= 3 ? 'gold' : level === 2 ? 'silver' : null;
  return h(
    'span',
    { class: `pill ${tier || ''}` },
    tier ? (tier === 'gold' ? '🥇' : '🥈') : h('i', { class: 'livedot sm off' }),
    `Nível ${level ?? '—'}`,
  );
}

function subscriptionBadge(a) {
  const tier = a.gamepass;
  if (tier != null) {
    const ultimate = /ultimate/i.test(tier);
    return h('span', { class: `pill gp${ultimate ? ' ultimate' : ''}` }, icon('xbox'), prettyTier(tier));
  }
  if (a.balance != null) return h('span', { class: 'pill outline' }, 'Sem Game Pass');
  return null;
}

function tierBar(t) {
  const done = t.progress >= t.max;
  const fraction = Math.min(1, Math.max(0, t.progress / t.max));
  return h(
    'div',
    { class: 'tierbar' },
    h(
      'div',
      { class: 'tl' },
      h('span', { class: 'ellip' }, t.next ? `${medal(t.tier)}${t.tier} → ${medal(t.next)}${t.next}` : `${medal(t.tier)}${t.tier} · nível máximo`),
      h('span', { class: `fig${done ? ' done' : ''}` }, `${money(t.progress)} / ${money(t.max)}`),
    ),
    h('div', { class: 'bar fat tier' }, h('i', { style: { width: `${fraction * 100}%` } })),
    !done && t.next && h('div', { class: 'caption muted' }, `Faltam ${money(t.max - t.progress)} pontos neste mês`),
  );
}

const copyEmail = async (email) => toast((await copyText(email)) ? 'E-mail copiado' : 'Não foi possível copiar', 'ok');

function emailLine(a) {
  return a.email && h('button', { class: 'email ellip', title: 'Copiar e-mail', onClick: () => copyEmail(a.email) }, a.email);
}

/** The app's long press on a card (right-click on a computer): ask to remove the account. */
function longPress(el, fn) {
  let timer = null;
  let sx = 0;
  let sy = 0;
  let fired = false;
  const cancel = () => { clearTimeout(timer); timer = null; };
  el.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    if (e.target.closest('button, a, input, [role="switch"]')) return;
    fired = false;
    sx = e.clientX;
    sy = e.clientY;
    timer = setTimeout(() => { timer = null; fired = true; fn(); }, 500);
  });
  el.addEventListener('pointermove', (e) => { if (timer && Math.hypot(e.clientX - sx, e.clientY - sy) > 10) cancel(); });
  for (const ev of ['pointerup', 'pointercancel', 'pointerleave']) el.addEventListener(ev, cancel);
  el.addEventListener('click', (e) => { if (fired) { e.stopPropagation(); e.preventDefault(); fired = false; } }, true);
  el.addEventListener('contextmenu', (e) => { e.preventDefault(); cancel(); fn(); });
  return el;
}
function accountCard(a) {
  const waiting = a.unclaimed || 0;
  const stale = !!a.staleReason;
  const claiming = S.claimingXuid === a.xuid;

  const bal = h('div', { class: `balance${stale ? ' danger' : ''}` });
  bal._v = lastBalance.get(a.xuid) ?? null;
  countUp(bal, a.balance);
  lastBalance.set(a.xuid, a.balance);

  const card = h(
    'div',
    { class: `card acct${a.farming ? ' farming' : ''}` },
    h(
      'div',
      { class: 'top' },
      avatar(a, a.farming ? 'ring-on' : ''),
      h(
        'div',
        { class: 'grow col' },
        h(
          'div',
          { class: 'name' },
          h('span', { class: 'row-title ellip' }, a.gamertag || a.xuid),
          h('span', { class: 'gscore' }, h('b', null, 'G'), money(a.gamerscore)),
        ),
        emailLine(a),
      ),
      h('button', { class: 'smallbtn', title: 'Congelar conta', 'aria-label': 'Congelar conta', onClick: () => act.toggleFrozen(a.xuid) }, h('span', { class: 'snow' }, '❄')),
      switchEl(a.farming, () => act.toggleFarm(a.xuid), `Farm de ${a.gamertag}`),
    ),
    h('div', { class: 'badges' }, levelBadge(a.level, stale), subscriptionBadge(a)),
    bal,
    h('div', { class: 'caption muted' }, 'pontos Rewards'),
    a.lifetime != null &&
      h('div', { class: 'lifetime' }, h('span', { class: 'row-fig t2' }, money(a.lifetime)), h('span', { class: 'caption muted' }, 'pontos lifetime')),
    a.tierProgress && tierBar(a.tierProgress),
    a.memberSince && h('div', { class: 'caption muted', style: { marginTop: '8px' } }, `Membro do Rewards desde ${brDate(a.memberSince)}`),
    h(
      'div',
      { class: 'actions' },
      h('button', { class: 'btn primary card-action', onClick: async () => { await act.select(a.xuid); location.hash = `#/store/${a.xuid}`; } }, icon('store'), 'Loja'),
      h('button', { class: 'btn', onClick: async () => { await act.select(a.xuid); location.hash = `#/missions/${a.xuid}`; } }, icon('missions'), a.missionsOpen != null ? `Missões · ${a.missionsOpen}` : 'Missões'),
    ),
    a.farming && farmBox(a),
    a.farming && presenceProblem(a),
    stale && h('div', { class: 'caption danger ellip', style: { marginTop: '8px' } }, a.staleReason),
    waiting > 0 &&
      h('button', { class: 'claimbtn', disabled: claiming, onClick: () => act.claim(a.xuid) }, claiming ? 'Resgatando…' : `Resgatar ${money(waiting)} pontos`),
  );
  return longPress(card, () => act.signOut(a.xuid));
}

function frozenCard(a) {
  return longPress(h(
    'div',
    { class: 'acct-frozen', style: { position: 'relative' } },
    avatar(a, 'dim'),
    h(
      'div',
      { class: 'grow col' },
      h('span', { class: 'row-title t2 ellip' }, a.gamertag || a.xuid),
      emailLine(a),
      a.balance != null && h('span', { class: 'caption-strong' }, `${money(a.balance)} pontos · antes de congelar`),
      a.lifetime != null && h('span', { class: 'caption t2' }, `${money(a.lifetime)} pontos lifetime`),
      a.memberSince && h('span', { class: 'caption muted' }, `Membro desde ${brDate(a.memberSince)}`),
      h('span', { class: 'caption ice' }, '❄ Congelada · nada é enviado'),
    ),
    h('button', { class: 'ghostline ice', style: { borderColor: 'rgba(143,211,255,.5)' }, onClick: () => act.toggleFrozen(a.xuid) }, 'Descongelar'),
  ), () => act.signOut(a.xuid));
}

// ------------------------------------------------------- farm progress

function strip(titles) {
  const shown = titles.slice(0, 8);
  const el = h('div', { class: 'strip', style: { width: `${26 + 13 * (shown.length - 1)}px` } });
  shown.forEach((t, i) => el.append(h('div', { class: 't', style: { left: `${13 * i}px` } }, gameArt(t))));
  return el;
}

function farmBox(a) {
  const titles = S.farmTitles;
  const el = h(
    'div',
    { class: 'farmbox', dataset: { xuid: a.xuid } },
    h(
      'div',
      { class: 'fh' },
      titles.length > 0 && strip(titles),
      h('div', { class: 'grow' }, h('div', { class: 'caption-strong' }, 'Missão de 15 min'), h('div', { class: 'caption muted' }, `${titles.length || 8} jogos em presença`)),
      h('div', { class: 'remaining' }, h('div', { class: 'row-fig', dataset: { role: 'clock' } }), h('div', { class: 'caption-strong green', dataset: { role: 'pct' } })),
    ),
    h('div', { class: 'bar fat linear' }, h('i', { dataset: { role: 'fill' } })),
  );
  paintFarm(el, a);
  return el;
}

function paintFarm(el, a) {
  const total = Math.max(1, a.farmTotal ?? 900);
  const since = a.farmAt ? (performance.now() - a.farmAt) / 1000 : 0;
  const elapsed = Math.min(total, Math.max(0, (a.farmElapsed ?? 0) + since));
  const remaining = Math.max(0, total - elapsed);
  const k = elapsed / total;
  const q = (r) => el.querySelector(`[data-role="${r}"]`);
  q('clock').textContent = clock(remaining);
  q('clock').style.color = remaining === 0 ? 'var(--green)' : '';
  q('pct').textContent = `${Math.floor(k * 100)}%`;
  q('fill').style.width = `${k * 100}%`;
}

/** Called every second: the farm's clock counts from the last real number. */
export function tickFarm() {
  document.querySelectorAll('.farmbox').forEach((el) => {
    const a = byXuid(el.dataset.xuid);
    if (a) paintFarm(el, a);
  });
}

/**
 * Why the farm's last ping did not register, in the user's words; nothing
 * while Xbox accepts it. A 403 is what an account signed in before the
 * Windows sign-in gets: its token's device is not a Windows device.
 */
function presenceProblem(a) {
  const code = a.farmPresenceHttp ? ` (${a.farmPresenceHttp})` : '';
  const text =
    a.farmPresence === 'denied' && a.farmPresenceHttp === 403
      ? 'Xbox recusou a presença (403) — remova a conta e entre de novo'
      : {
          denied: `Xbox recusou a presença${code} — o farm não está contando`,
          rejected: `Xbox rejeitou a presença${code} — o farm não está contando`,
          not_sent: 'Presença não enviada — sem conexão com a Xbox',
          unknown: `Xbox não confirmou a presença${code}`,
        }[a.farmPresence];
  return text ? h('div', { class: 'caption danger clamp2', style: { marginTop: '8px' } }, text) : null;
}

// ------------------------------------------------------------- welcome

function welcome(enter) {
  return h(
    'div',
    { class: 'welcome' },
    h('div', { class: `logo big${enter ? ' rise' : ''}` }, icon('xbox')),
    h('h1', null, 'Xbox ', h('span', { class: 'green' }, 'Rewards')),
    h('div', { class: 't2' }, 'Jogos. Presença. Recompensas.'),
    h(
      'p',
      { class: S.fatal ? 'danger' : 't2', style: { margin: '32px 0 24px' } },
      S.fatal || 'Entre com a sua conta Microsoft para ver o saldo do Rewards, resgatar os pontos pendentes e rodar o farm de presença.',
    ),
    h('button', { class: 'btn primary lg', onClick: () => startLogin() }, S.fatal ? 'Tentar de novo' : 'Entrar com a Microsoft'),
  );
}
