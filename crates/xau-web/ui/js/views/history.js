// Histórico: every Rewards level-up the server has seen, newest first, one
// banner each, grouped by day. Only accounts that levelled up appear.

import { h, money, plural, timeOf, dayOf, dayLabel } from '../dom.js';
import { api } from '../api.js';
import { header, skeleton, stateBlock, avatar } from '../ui.js';

export function mount(root) {
  const page = h('div', { class: 'page' });
  root.append(page);
  let alive = true;
  let entries = null;
  let error = null;

  function render() {
    page.replaceChildren(
      header({
        back: '#/',
        title: 'Histórico de níveis',
        sub: entries ? (entries.length ? `${entries.length} ${plural(entries.length, 'subida', 'subidas')} de nível` : 'Nenhuma subida ainda') : null,
      }),
      body(),
    );
  }

  function body() {
    if (error) return stateBlock({ title: 'Não foi possível ler o histórico', detail: error, tone: 'danger' });
    if (!entries) return h('div', { class: 'stack-sm' }, skeleton(520, 84), skeleton(520, 84), skeleton(520, 84));
    if (!entries.length) {
      return stateBlock({
        glyph: '🏆',
        title: 'Nenhuma conta subiu de nível ainda',
        detail: 'Quando uma conta subir de nível no Rewards, ela aparece aqui com o horário.',
      });
    }
    // Newest first, a day header wherever the day changes.
    const out = [];
    let day = null;
    entries.forEach((e, i) => {
      const d = dayOf(e.at);
      if (d !== day) {
        day = d;
        out.push(h('div', { class: 'daylabel' }, dayLabel(d)));
      }
      out.push(banner(e, i));
    });
    return h('div', { class: 'stack-sm' }, out);
  }

  render();
  api
    .levelLog()
    .then((l) => (entries = l))
    .catch((e) => (error = e.message))
    .finally(() => alive && render());
  return { update() {}, destroy: () => (alive = false) };
}

/** One level-up: who, from which level to which, their best balance, and when. */
function banner(e, i) {
  return h(
    'div',
    { class: 'lvl rise', style: { '--i': Math.min(i, 8) } },
    avatar(e, 'xs gold'),
    h(
      'div',
      { class: 'grow' },
      h('div', { class: 'row-title ellip' }, e.gamertag),
      h('div', { class: 'rowx gap-sm', style: { marginTop: '2px' } }, h('span', { class: 'stamp' }, `Nível ${e.from} → ${e.to}`), h('span', { class: 'caption t2' }, timeOf(e.at))),
    ),
    h('div', { class: 'peak' }, h('div', { class: 'row-fig' }, money(e.peak)), h('div', { class: 'caption muted' }, 'maior saldo')),
  );
}
