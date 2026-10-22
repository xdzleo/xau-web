// Loja: the Microsoft Store's games. Search, filters (Microsoft-published,
// no free games, a price range), favorites kept in this browser, and the
// purchase, which opens the Store's own checkout already signed in.
//
// The server reads, filters and caches the Store's pages (the same
// `store_browse` the app uses); this screen only asks for "about 36 games
// that pass" and shows them as they come.

import { h, put, img, money, debounce } from '../dom.js';
import { api } from '../api.js';
import { S, live, active, byXuid, prefs } from '../state.js';
import { modal, toast, avatar } from '../ui.js';
import * as act from '../actions.js';

const FAVORITES = 'storeFavorites';
const favorites = () => prefs.get(FAVORITES, []);
const isFavorite = (id) => favorites().some((p) => p.product_id === id);

function toggleFavorite(p) {
  const list = favorites();
  const at = list.findIndex((x) => x.product_id === p.product_id);
  if (at >= 0) list.splice(at, 1);
  else list.unshift(p);
  prefs.set(FAVORITES, list);
}

/** `249`, `249,90`, `R$ 1.249,00` -> reais; blank or junk -> null. */
function parseMoney(text) {
  const digits = String(text).replace(/[^\d,.]/g, '');
  if (!/\d/.test(digits)) return null;
  const n = digits.includes(',') ? digits.replace(/\./g, '').replace(',', '.') : digits;
  const v = Number(n);
  return Number.isFinite(v) ? v : null;
}

export function mount(root, { xuid }) {
  const page = h('div', { class: 'page' });
  root.append(page);

  let alive = true;
  let query = '';
  let shownQuery = '';
  const filter = { microsoft: false, hideFree: false, min: '', max: '' };
  let products = [];
  const seen = new Set();
  let cursor; // undefined: not read yet; null: the end
  let loading = true;
  let error = null;
  let rid = 0; // a newer search or filter makes older answers moot

  const filterActive = () => filter.microsoft || filter.hideFree || filter.min.trim() !== '' || filter.max.trim() !== '';
  const wantsPaid = () => filter.hideFree || (parseMoney(filter.min) ?? 0) > 0;
  const account = () => (xuid && byXuid(xuid)) || active() || null;

  // ---- the parts that must not be redrawn while the user types
  const search = h('input', {
    class: 'searchfield',
    type: 'search',
    placeholder: 'Buscar jogos na loja',
    autocomplete: 'off',
    'aria-label': 'Buscar jogos na loja',
    onInput: (e) => {
      query = e.target.value;
      typed();
    },
    onKeydown: (e) => e.key === 'Enter' && e.target.blur(),
  });
  const chipMs = h('button', { class: 'chip', onClick: () => { filter.microsoft = !filter.microsoft; chips(); fetchPage(true); } }, 'Microsoft');
  const chipFree = h('button', { class: 'chip', onClick: () => { filter.hideFree = !filter.hideFree; chips(); fetchPage(true); } }, 'Sem grátis');
  const fieldMin = h('input', { class: 'moneyfield', inputmode: 'decimal', placeholder: 'Mín R$', 'aria-label': 'Preço mínimo', onInput: (e) => { filter.min = e.target.value.replace(/[^\d,.]/g, '').slice(0, 8); e.target.value = filter.min; typedPrice(); } });
  const fieldMax = h('input', { class: 'moneyfield', inputmode: 'decimal', placeholder: 'Máx R$', 'aria-label': 'Preço máximo', onInput: (e) => { filter.max = e.target.value.replace(/[^\d,.]/g, '').slice(0, 8); e.target.value = filter.max; typedPrice(); } });
  const clearBtn = h('button', { class: 'chip', title: 'Limpar filtros', 'aria-label': 'Limpar filtros', onClick: () => { Object.assign(filter, { microsoft: false, hideFree: false, min: '', max: '' }); fieldMin.value = ''; fieldMax.value = ''; chips(); fetchPage(true); } }, '✕');
  const pickerSlot = h('span');
  const chipsRow = h('div', { class: 'chips' }, chipMs, chipFree, fieldMin, fieldMax, clearBtn, pickerSlot);
  const bannerSlot = h('div');
  const favSlot = h('div');
  const titleEl = h('div', { class: 'section', style: { marginTop: '16px' } });
  const grid = h('div', { class: 'sgrid' });
  const sentinel = h('div', { class: 'loadmore' });

  page.append(
    h('div', { class: 'stickyhead' }, h('div', { class: 'rowx gap-sm' }, h('a', { class: 'back', href: '#/', 'aria-label': 'Voltar' }, '‹'), search), chipsRow, bannerSlot),
    favSlot,
    titleEl,
    grid,
    sentinel,
  );

  function chips() {
    chipMs.classList.toggle('on', filter.microsoft);
    chipFree.classList.toggle('on', filter.hideFree);
    clearBtn.hidden = !filterActive();
  }

  // ---- the account the purchase is made with, and its live Rewards
  function renderBanner() {
    const a = account();
    const accounts = live();
    put(pickerSlot, 
      accounts.length > 1 &&
        h(
          'select',
          {
            class: 'select',
            style: { width: 'auto', height: '36px', borderRadius: '50px' },
            'aria-label': 'Conta',
            onChange: async (e) => {
              await act.select(e.target.value);
              location.hash = `#/store/${e.target.value}`;
            },
          },
          accounts.map((x) => h('option', { value: x.xuid, selected: x.xuid === a?.xuid }, x.gamertag)),
        ),
    );
    put(bannerSlot, 
      a
        ? h(
            'a',
            { class: 'glass', href: `#/missions/${a.xuid}`, style: { textDecoration: 'none', color: 'inherit' } },
            avatar(a, 'sm'),
            h(
              'div',
              { class: 'grow' },
              h('div', { class: 'rowx gap-sm' }, h('i', { class: `livedot${a.balance == null ? ' off' : ''}` }), h('span', { class: 'caption-strong green' }, 'REWARDS · AO VIVO')),
              h('div', { class: 'bal' }, money(a.balance)),
              h('div', { class: 'caption t2 ellip' }, `${a.gamertag} · Nível ${a.level ?? '—'}`),
            ),
            h('div', { class: 'col', style: { alignItems: 'flex-end' } }, h('span', { class: 'caption t2' }, 'Missões'), h('span', { class: 'row-fig' }, `${a.missionsOpen ?? '—'} ›`)),
          )
        : h('div', { class: 'caption muted', style: { marginTop: '12px' } }, 'Entre com uma conta para comprar. A busca e os preços funcionam sem login.'),
    );
  }

  // ---- the list
  function card(p) {
    const fav = isFavorite(p.product_id);
    return h(
      'button',
      { class: 'pcard', onClick: () => openProduct(p) },
      h(
        'div',
        { class: 'cover' },
        img(p.image, p.title),
        p.pass && h('span', { class: 'gp' }, 'GAME PASS'),
        h(
          'span',
          {
            class: `heart${fav ? ' on' : ''}`,
            role: 'button',
            'aria-label': fav ? 'Tirar dos favoritos' : 'Favoritar',
            onClick: (e) => {
              e.stopPropagation();
              toggleFavorite(p);
              renderFavs();
              renderGrid();
            },
          },
          fav ? '♥' : '♡',
        ),
      ),
      h('div', { class: 'ttl clamp2' }, p.title),
      priceLine(p),
    );
  }

  function renderFavs() {
    const list = favorites();
    put(favSlot, 
      !shownQuery && list.length > 0 &&
        h('div', null, h('div', { class: 'section', style: { margin: '16px 0 8px' } }, '♥ Favoritos'), h('div', { class: 'favrow' }, list.map(card))),
    );
  }

  function renderGrid() {
    titleEl.textContent = shownQuery ? `Resultados para “${shownQuery}”` : 'Em alta na Microsoft Store';
    const nodes = products.map(card);
    if (error && products.length === 0) nodes.push(h('div', { class: 'danger', style: { gridColumn: '1 / -1' } }, error));
    if (filterActive() && !loading && products.length === 0 && !error) {
      nodes.push(h('div', { class: 'muted', style: { gridColumn: '1 / -1' } }, 'Nenhum jogo desta lista passa pelos filtros.'));
    }
    if (loading) for (let i = 0; i < (products.length ? 3 : 9); i++) nodes.push(h('div', { class: 'skeleton', style: { aspectRatio: '.72', borderRadius: 'var(--r-card)' } }));
    grid.replaceChildren(...nodes);
  }

  async function fetchPage(reset) {
    if (reset) {
      rid++;
      products = [];
      seen.clear();
      cursor = undefined;
      shownQuery = query.trim();
      error = null;
      put(favSlot, );
      renderFavs();
    } else if (loading || cursor == null) {
      return;
    }
    const my = rid;
    loading = true;
    renderGrid();
    try {
      const out = await api.storeBrowse({
        query: shownQuery,
        paid: wantsPaid(),
        cursor: reset ? '' : cursor,
        microsoft: filter.microsoft,
        hideFree: filter.hideFree,
        min: parseMoney(filter.min),
        max: parseMoney(filter.max),
      });
      if (my !== rid) return;
      for (const p of out.products) {
        if (!seen.has(p.product_id)) {
          seen.add(p.product_id);
          products.push(p);
        }
      }
      cursor = out.cursor ?? null;
    } catch (e) {
      if (my === rid) {
        error = e.message;
        cursor = null; // one failure ends the paging; a new search starts over
      }
    } finally {
      if (my === rid) {
        loading = false;
        alive && renderGrid();
        alive && more();
      }
    }
  }

  /** A narrow filter can come back with less than a screen: keep asking until it fills. */
  function more() {
    if (cursor != null && !loading && (products.length < 18 || sentinelVisible())) fetchPage(false);
  }
  const sentinelVisible = () => sentinel.getBoundingClientRect().top < window.innerHeight + 600;

  // Typing waits for a pause; a chip answers at once (its pages are usually cached).
  const searchLater = debounce(() => fetchPage(true), 300);
  function typed() {
    const q = query.trim();
    if (q.length === 1) return; // the Store needs two letters
    if (q === '') fetchPage(true);
    else searchLater();
  }
  const typedPrice = () => {
    chips();
    searchLater();
  };

  const onScroll = () => alive && more();
  window.addEventListener('scroll', onScroll, { passive: true });

  // Real time: the balance is re-read while the store is in front.
  const timer = setInterval(() => document.visibilityState === 'visible' && act.refresh({ quiet: true }), 15_000);

  // ---- a game, and the way to buy it
  function openProduct(p) {
    const accounts = live();
    let buyer = account()?.xuid || accounts[0]?.xuid || null;
    const heart = h('button', { class: `heartbtn${isFavorite(p.product_id) ? ' on' : ''}`, 'aria-label': 'Favoritar', onClick: () => {
      toggleFavorite(p);
      heart.classList.toggle('on', isFavorite(p.product_id));
      heart.textContent = isFavorite(p.product_id) ? '♥' : '♡';
      renderFavs();
      renderGrid();
    } }, isFavorite(p.product_id) ? '♥' : '♡');
    const buy = h('button', { class: 'btn primary', style: { flex: 1 }, onClick: () => purchase(p, buyer, buy, m) }, 'Comprar na Microsoft Store');
    const m = modal(
      h(
        'div',
        { class: 'sheet', role: 'dialog', 'aria-label': p.title },
        h('div', { class: 'grip' }),
        h(
          'div',
          { class: 'pdetail' },
          h('div', { class: 'cover' }, img(p.image, p.title)),
          h(
            'div',
            { class: 'grow col gap-sm' },
            h('div', { class: 'section' }, p.title),
            p.publisher && h('div', { class: 'caption t2' }, p.publisher),
            p.rating != null && h('div', { class: 'caption-strong' }, `★ ${Number(p.rating).toFixed(1)}`),
            priceLine(p),
            p.pass && h('div', { class: 'caption-strong green' }, `Incluso no ${p.pass}`),
          ),
        ),
        accounts.length > 1 &&
          h(
            'div',
            { style: { marginTop: '16px' } },
            h('div', { class: 'caption t2', style: { marginBottom: '4px' } }, 'Comprar com'),
            h('select', { class: 'select', onChange: (e) => (buyer = e.target.value) }, accounts.map((x) => h('option', { value: x.xuid, selected: x.xuid === buyer }, x.gamertag))),
          ),
        h('div', { class: 'buyrow' }, heart, buy),
        h(
          'div',
          { class: 'caption muted', style: { marginTop: '8px' } },
          S.info?.nativeWindows
            ? 'Abre a página oficial do jogo numa janela no computador do servidor, já com a sua conta. Você confirma e paga lá.'
            : 'Abre a página oficial do jogo numa nova aba. Você entra, confirma e paga lá, com a sua conta Microsoft.',
        ),
      ),
      { bottom: true },
    );
  }

  async function purchase(p, xuid, btn, m) {
    btn.disabled = true;
    try {
      if (S.info?.nativeWindows && xuid) {
        await api.checkout(xuid, p.product_id);
        toast('A página de compra abriu numa janela no computador do servidor.');
      } else {
        // No native window (or no account): the public product page, where
        // the user signs in and pays exactly as on the web.
        const r = await api.storePurchaseUrl(p.product_id);
        window.open(r.url, '_blank', 'noopener,noreferrer');
      }
      m.close();
    } catch (e) {
      toast(e.message, 'err');
    } finally {
      btn.disabled = false;
    }
  }

  chips();
  renderBanner();
  renderFavs();
  renderGrid();
  fetchPage(true);

  return {
    update: () => alive && renderBanner(),
    destroy() {
      alive = false;
      rid++;
      clearInterval(timer);
      window.removeEventListener('scroll', onScroll);
    },
  };
}

function priceLine(p) {
  return h(
    'div',
    { class: `price${p.was ? ' sale' : ''}` },
    p.was && h('s', null, p.was),
    p.was ? h('b', null, p.price ?? '—') : h('span', null, p.price ?? '—'),
  );
}
