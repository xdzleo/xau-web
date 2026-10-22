// Vantagens: the perks Rewards gives (Discord Nitro, Store codes...). One
// card per reward, its codes in the detail, and the history of every code
// ever claimed.
//
// Claiming cannot be undone, so it is always confirmed. The reward inside an
// offer is minted by the claim that consumes it, and the server writes it to
// a CSV before this screen ever sees it: a code that exists only in a toast
// is a code that can be lost.

import { h, put, icon, plural, prettyType, prettyDate, copyText, sleep } from '../dom.js';
import { api } from '../api.js';
import { header, skeleton, stateBlock, confirmDialog } from '../ui.js';

export function mount(root, params) {
  const page = h('div', { class: 'page' });
  root.append(page);
  const screen = { list: List, history: History, detail: Detail }[params.view](page, params);
  return screen;
}

// ------------------------------------------------------------- grouping

/**
 * Collapse every account's list into one row per reward. Names come from
 * the content feed and one account's response can be missing a title the
 * next one carries, so the first non-null wins.
 */
function group(accounts) {
  const byId = new Map();
  for (const a of accounts) {
    for (const o of a.offers) {
      const cur = byId.get(o.offerId);
      byId.set(o.offerId, {
        offerId: o.offerId,
        title: (cur?.title && cur.title.trim()) || o.title || prettyType(o.resourceType),
        description: cur?.description ?? o.description,
        resourceType: cur?.resourceType ?? o.resourceType,
        accounts: (cur?.accounts ?? 0) + 1,
        available: (cur?.available ?? 0) + (o.available ? 1 : 0),
        claimed: (cur?.claimed ?? 0) + (o.claimed ? 1 : 0),
      });
    }
  }
  // Anything still claimable first: that is what the screen is for.
  return [...byId.values()].sort((a, b) => Number(b.available > 0) - Number(a.available > 0) || a.title.localeCompare(b.title));
}

const status = (g) =>
  g.available > 0 && g.claimed > 0
    ? `${g.available} a resgatar · ${g.claimed} de ${g.accounts} já resgatadas`
    : g.available > 0
      ? `${plural(g.available, '1 conta pode', `${g.available} contas podem`)} resgatar`
      : g.claimed > 0
        ? `resgatada em ${plural(g.claimed, '1 conta', `${g.claimed} contas`)}`
        : 'indisponível nas contas conectadas';

const claimText = (accounts) =>
  `Vale para ${plural(accounts, 'a conta conectada', `as ${accounts} contas conectadas`)}. O resgate não pode ser desfeito: cada vantagem é consumida na conta onde for resgatada. Os links e códigos recebidos ficam salvos em CSV no computador do servidor, um arquivo por vantagem.`;

// ----------------------------------------------------------------- list

function List(page) {
  let accounts = null;
  let error = null;
  let claiming = false;
  let result = null;
  let alive = true;

  async function load() {
    try {
      accounts = await api.offersAll();
      error = null;
    } catch (e) {
      error = e.message;
    }
    alive && render();
  }

  async function claimAll(available, n) {
    if (!(await confirmDialog({ title: `Resgatar ${available} vantagens?`, text: claimText(n), confirm: 'Resgatar' }))) return;
    claiming = true;
    render();
    try {
      result = await api.claimOffers('');
      await load();
    } catch (e) {
      error = e.message;
    }
    claiming = false;
    alive && render();
  }

  function render() {
    const failed = (accounts || []).filter((a) => a.error);
    const groups = group(accounts || []);
    const available = groups.reduce((n, g) => n + g.available, 0);
    const n = (accounts || []).length - failed.length;
    put(page, 
      header({
        back: '#/',
        title: 'Vantagens',
        sub: `${available} a resgatar em ${n} ${plural(n, 'conta', 'contas')}`,
        // Always reachable: the history is read from disk, not the network.
        right: h('a', { class: 'btn', href: '#/offers/history' }, 'Resgatados'),
      }),
      available > 0 &&
        h('button', { class: 'btn primary fill', style: { background: claiming ? 'var(--surface-high)' : '', marginBottom: '12px' }, disabled: claiming, onClick: () => claimAll(available, n) }, claiming ? 'Resgatando…' : `Resgatar todas (${available})`),
      body(groups, failed),
    );
  }

  function body(groups, failed) {
    if (error) return stateBlock({ title: 'Não foi possível ler as vantagens', detail: error, actionLabel: 'Tentar de novo', onAction: load, tone: 'danger' });
    if (!accounts) return h('div', { class: 'stack-sm' }, skeleton(520, 64), skeleton(520, 64), skeleton(520, 64));
    return h(
      'div',
      { class: 'stack-sm' },
      result && resultBanner(result),
      // An account whose list failed to load is named, not silently dropped.
      failed.map((a) => h('div', { class: 'caption danger clamp2' }, `${a.gamertag}: ${a.error}`)),
      groups.map(offerCard),
      !groups.length && stateBlock({ title: 'Nenhuma vantagem', detail: 'O Rewards não listou nenhuma vantagem para as contas conectadas.' }),
    );
  }

  render();
  load();
  return { update() {}, destroy: () => (alive = false) };
}

function resultBanner(r) {
  return h(
    'div',
    { class: 'banner-ok' },
    h('div', { class: 'row-title green' }, `${r.claimed.length} vantagens resgatadas`),
    r.indexCsv && h('div', { class: 'caption muted', style: { marginTop: '4px', overflowWrap: 'anywhere' } }, `CSV: ${r.indexCsv}`),
    r.failures.slice(0, 3).map((f) => h('div', { class: 'caption danger ellip' }, typeof f === 'string' ? f : `${f.gamertag}: ${f.error}`)),
  );
}

function offerCard(g) {
  const used = g.available === 0 && g.claimed > 0;
  return h(
    'a',
    { class: 'ocard', href: `#/offers/${g.offerId}` },
    used ? h('span', { class: 'checkic' }, icon('check')) : h('span', { class: 'radio' }),
    h(
      'div',
      { class: 'grow' },
      // The real name, from the content feed.
      h('div', { class: `row-title ellip${g.available > 0 ? '' : ' t2'}` }, g.title),
      h('div', { class: `caption ellip ${g.available > 0 ? 'green' : 'muted'}`, style: { marginTop: '2px' } }, status(g)),
    ),
    h('span', { class: 'chev' }, '›'),
  );
}

// --------------------------------------------------------------- detail

/** Every code one reward minted, read back from the CSVs on the server. */
function Detail(page, { id }) {
  let alive = true;
  let group_ = null;
  let rows = null;
  let error = null;
  let notice = null;
  let claiming = false;
  let copied = null;

  async function loadGroup() {
    try {
      group_ = group(await api.offersAll()).find((g) => g.offerId === id) || null;
    } catch {
      /* the codes below still read from disk */
    }
  }
  async function loadRows() {
    try {
      rows = await api.offerCodes(id);
      error = null;
    } catch (e) {
      error = e.message;
    }
  }

  async function claim() {
    const g = group_;
    if (!(await confirmDialog({ title: `Resgatar ${g.title}?`, text: claimText(g.available), confirm: 'Resgatar' }))) return;
    claiming = true;
    notice = null;
    render();
    try {
      const r = await api.claimOffers(id);
      notice = r.failures.length ? `falhou: ${typeof r.failures[0] === 'string' ? r.failures[0] : r.failures[0].error}` : plural(r.claimed.length, '1 resgate feito', `${r.claimed.length} resgates feitos`);
    } catch (e) {
      notice = `não deu para resgatar: ${e.message}`;
    }
    await Promise.all([loadRows(), loadGroup()]);
    claiming = false;
    alive && render();
  }

  async function copyAll(list) {
    // One code per line: the shape you can paste into a notes app and work
    // down, and the shape a browser takes one line at a time.
    await copyText(list.join('\n'));
    copied = null;
    notice = plural(list.length, '1 código copiado', `${list.length} códigos copiados`);
    render();
  }

  async function download() {
    try {
      // Read again here, not when the page loaded: a claim in between
      // belongs in the file the user is saving.
      const f = await api.offerCsv(id, group_?.title || '');
      const blob = new Blob(['﻿', f.text], { type: 'text/csv;charset=utf-8' });
      const a = h('a', { href: URL.createObjectURL(blob), download: f.fileName });
      document.body.append(a);
      a.click();
      a.remove();
      notice = 'CSV salvo';
    } catch (e) {
      notice = `não deu para salvar: ${e.message}`;
    }
    render();
  }

  function render() {
    const g = group_;
    const title = g?.title || 'Vantagem';
    const list = (rows || []).map((r) => r.code).filter((c) => c && c.trim());
    const sub = notice ?? (rows ? plural(rows.length, '1 código salvo', `${rows.length} códigos salvos`) : 'lendo o CSV…');
    const bad = notice?.startsWith('falhou') || notice?.startsWith('não deu');
    put(page, 
      header({ back: '#/offers', title, sub: h('span', { style: { color: bad ? 'var(--danger)' : notice ? 'var(--green)' : '' } }, sub) }),
      (g?.available > 0 || claiming) &&
        h('button', { class: 'btn primary fill', style: { marginBottom: '12px' }, disabled: claiming, onClick: claim }, claiming ? 'Resgatando…' : g.available === 1 ? 'Resgatar' : `Resgatar em ${g.available} contas`),
      list.length > 0 &&
        h('div', { class: 'actions2', style: { marginBottom: '12px' } }, h('button', { class: 'btn', onClick: () => copyAll(list) }, `Copiar tudo (${list.length})`), h('button', { class: 'btn', onClick: download }, 'Baixar CSV')),
      body(),
    );
  }

  function body() {
    if (error) return stateBlock({ title: 'Não foi possível ler o CSV', detail: error, tone: 'danger' });
    if (!rows) return h('div', { class: 'stack-sm' }, skeleton(520, 64), skeleton(520, 64));
    if (!rows.length) {
      return stateBlock({ title: 'Nenhum código ainda', detail: 'Um código só existe depois do resgate: é o resgate que o cria. Resgates feitos fora do app não aparecem aqui.' });
    }
    return h(
      'div',
      { class: 'stack-sm' },
      h('div', { class: 'codehead' }, h('span', null, 'CÓDIGO / LINK'), h('span', null, 'DATA')),
      rows.map((r, i) => codeRow(r, copied === i, false, async () => {
        await copyText(r.code);
        copied = i;
        render();
        await sleep(1600);
        if (copied === i) {
          copied = null;
          alive && render();
        }
      })),
    );
  }

  render();
  Promise.all([loadRows(), loadGroup()]).then(() => alive && render());
  return { update() {}, destroy: () => (alive = false) };
}

function codeRow(r, justCopied, showAccount, onCopy) {
  return h(
    'button',
    { class: 'code', onClick: onCopy },
    // Not one truncated line: a promo URL is the thing the user came for,
    // and a link cut off at the ellipsis cannot be read back to check it.
    h('div', { class: `val clamp3${r.code?.trim() ? '' : ' blank'}` }, r.code?.trim() ? r.code : '—'),
    h(
      'div',
      { class: 'meta caption' },
      h('span', { class: 't2 ellip' }, showAccount && r.gamertag ? `${r.gamertag} · ${prettyDate(r.claimedDate)}` : prettyDate(r.claimedDate)),
      h('span', { class: justCopied ? 'green caption-strong' : 'muted' }, justCopied ? 'copiado' : 'copiar'),
    ),
  );
}

// -------------------------------------------------------------- history

/**
 * Every code ever claimed, every account, every month, by reward. The live
 * list only knows what the service still returns for the accounts signed in
 * now; this is the record: the CSVs on the server, so a Discord link from
 * three renewals ago, or from an account since removed, is still here.
 */
function History(page) {
  let alive = true;
  let groups = null;
  let error = null;
  let copied = null;

  async function flash(key) {
    copied = key;
    alive && render();
    await sleep(1600);
    if (copied === key) {
      copied = null;
      alive && render();
    }
  }

  function render() {
    const n = groups ? groups.reduce((s, g) => s + g.codes.length, 0) : 0;
    put(page, 
      header({
        back: '#/offers',
        title: 'Resgatados',
        sub: groups ? `${plural(n, '1 código', `${n} códigos`)} em ${plural(groups.length, '1 vantagem', `${groups.length} vantagens`)}` : 'lendo os CSVs…',
      }),
      body(),
    );
  }

  function body() {
    if (error) return stateBlock({ title: 'Não foi possível ler os resgates', detail: error, tone: 'danger' });
    if (!groups) return h('div', { class: 'stack-sm' }, skeleton(520, 64), skeleton(520, 64));
    if (!groups.length) return stateBlock({ title: 'Nenhum resgate ainda', detail: 'Cada vantagem resgatada por este app fica salva aqui, com o código ou link, para sempre.' });
    return h(
      'div',
      { class: 'stack-sm' },
      groups.map((g, gi) => {
        const codes = g.codes.map((c) => c.code).filter((c) => c && c.trim());
        return [
          h(
            'div',
            { class: 'hist-group' },
            h('div', { class: 'grow' }, h('div', { class: 'row-title clamp2' }, g.title?.trim() || prettyType(g.resourceType)), h('div', { class: 'caption muted' }, plural(g.codes.length, '1 código', `${g.codes.length} códigos`))),
            h('button', { class: 'btn', onClick: async () => { await copyText(codes.join('\n')); flash(`${gi}:*`); } }, copied === `${gi}:*` ? 'Copiado' : 'Copiar tudo'),
          ),
          g.codes.map((c, ci) => codeRow(c, copied === `${gi}:${ci}`, true, async () => { await copyText(c.code); flash(`${gi}:${ci}`); })),
        ];
      }),
    );
  }

  render();
  api
    .offerHistory()
    .then((g) => (groups = g))
    .catch((e) => (error = e.message))
    .finally(() => alive && render());
  return { update() {}, destroy: () => (alive = false) };
}
