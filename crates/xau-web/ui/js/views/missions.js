// Missões: every Rewards activity of one account, drawn the way the
// xbox.com Rewards page draws it: day boxes for streaks, game covers for
// monthly packs, a bar for the rest, and a status line under it all.

import { h, icon, img, money, ptBr, daysLeft } from '../dom.js';
import { api } from '../api.js';
import { byXuid } from '../state.js';
import { header, avatar, skeleton, stateBlock } from '../ui.js';

export function mount(root, { xuid }) {
  const page = h('div', { class: 'page' });
  root.append(page);
  let alive = true;
  let missions = null;
  let error = null;

  const account = () => byXuid(xuid);

  function render() {
    const a = account();
    page.replaceChildren(
      header({
        back: '#/',
        title: a?.gamertag || 'Conta',
        sub: a ? a.email || `Nível ${a.level ?? '—'} · ${money(a.balance)} pontos` : null,
        lead: a && avatar(a, 'sm'),
      }),
      body(),
    );
  }

  function body() {
    if (error) return stateBlock({ title: 'Não foi possível ler as missões', detail: error, tone: 'danger' });
    if (!missions) return h('div', { class: 'stack-sm' }, skeleton(520, 120), skeleton(520, 120), skeleton(520, 90));
    if (!missions.length) return stateBlock({ title: 'Nenhuma missão', detail: 'O Rewards não listou nenhuma atividade para esta conta.' });

    const open = missions.filter((m) => m.open);
    const done = missions.filter((m) => m.done);
    // Neither open nor done: behind a requirement. The official page
    // still lists these, with what unlocks them.
    const locked = missions.filter((m) => !m.open && !m.done);
    const group = (label, list, tint) =>
      list.length
        ? [
            h('div', { class: 'groupheadline' }, h('span', { class: 'row-title', style: { color: tint } }, list.length), h('span', { class: 'section' }, label)),
            h('div', { class: 'stack-sm' }, list.map(missionCard)),
          ]
        : [];
    return h('div', null, ...group('Faltam', open, 'var(--green)'), ...group('Bloqueadas', locked, 'var(--muted)'), ...group('Concluídas', done, 'var(--muted)'));
  }

  render();
  api
    .missions(xuid)
    .then((m) => (missions = m))
    .catch((e) => (error = e.message))
    .finally(() => alive && render());

  return {
    update: () => alive && render(),
    destroy: () => (alive = false),
  };
}

// ---------------------------------------------------------------- card

function statusDot(done) {
  return h('span', { class: `sdot${done ? ' done' : ''}` }, done && icon('check'));
}

function missionCard(m) {
  const dim = m.locked || m.failed;
  const head = h('div', { class: 'mh' });
  if (m.kind === 'single') {
    if (m.thumbnail || m.locked) {
      head.append(h('div', { class: `thumb${m.locked ? ' locked' : ''}` }, img(m.thumbnail, ''), m.locked && icon('lock')));
    } else {
      head.append(statusDot(m.done));
    }
  }
  head.append(
    h(
      'div',
      { class: 'grow' },
      // The service does not always name an activity. Its own status label
      // is a truer fallback than a made-up title.
      h('div', { class: `row-title clamp2${m.done ? ' t2' : ''}` }, m.title || m.label || 'Atividade'),
      m.description && h('div', { class: 'caption t2 clamp2', style: { marginTop: '4px' } }, m.description),
    ),
  );
  if (m.kind === 'single' && m.points > 0 && !m.locked) {
    head.append(h('span', { class: `caption-strong ${m.done ? 'muted' : 'green'}` }, `+${money(m.points)}`));
  }

  const card = h('div', { class: 'mcard' }, head);

  if (m.kind === 'streak' && m.steps.length) {
    card.append(
      h(
        'div',
        { class: `steps${dim ? ' dim' : ''}` },
        m.steps.map((s, i) =>
          h(
            'div',
            { class: `step${s.done ? ' done' : ''}${i === m.current ? ' current' : ''}`, style: { flex: s.points > 0 || s.note ? 1.6 : 1 } },
            h('div', { class: 'sl' }, ptBr(s.label) || `Dia ${i + 1}`),
            s.points > 0 && h('div', { class: 'sp' }, `+${money(s.points)}`),
            s.note && h('div', { class: 'sn clamp2' }, ptBr(s.note)),
            statusDot(s.done),
          ),
        ),
      ),
    );
  } else if (m.kind === 'pack') {
    // A monthly pack's game slots: the cover of each game already played,
    // the slot number for the rest, and the next slot outlined.
    card.append(
      h(
        'div',
        { class: `pack${dim ? ' dim' : ''}` },
        m.steps.map((s, i) => h('div', { class: `slot${i === m.current ? ' current' : ''}` }, s.image ? img(s.image, '') : s.done ? statusDot(true) : String(i + 1))),
      ),
    );
  } else if (m.locked || (!m.done && m.progress >= 1 && m.progress <= 99)) {
    card.append(h('div', { style: { marginTop: '12px' } }, h('div', { class: 'bar' }, h('i', { style: { width: `${m.progress}%`, background: m.locked ? 'var(--muted)' : '' } })), !m.locked && h('div', { class: 'caption green', style: { marginTop: '4px' } }, `${m.progress}%`)));
  }

  const status = statusLine(m);
  if (status) card.append(status);
  return card;
}

const mentionsGamePass = (m) => [m.title, m.id, m.description].some((s) => /game ?pass/i.test(s || ''));

/**
 * The footer of the official card. Left, the time the period has left;
 * right, the outcome: "Requer assinatura Game Pass 🔒 +40",
 * "Tente na próxima semana ✕ +110" or "Resgatado ✓ 80".
 */
function statusLine(m) {
  let outcome = null;
  if (m.locked) {
    outcome = { text: m.requirement || (mentionsGamePass(m) ? 'Requer assinatura Game Pass' : m.label || 'Bloqueada'), glyph: 'lock', tone: 'var(--text2)' };
  } else if (m.failed) {
    outcome = { text: 'Tente na próxima semana', glyph: 'x', tone: 'var(--danger)' };
  } else if (m.claimed != null && m.claimed > 0) {
    outcome = { text: 'Resgatado', glyph: 'check', tone: 'var(--green)' };
  } else if (m.kind !== 'single' && m.target != null) {
    outcome = { text: `${m.count ?? 0}/${m.target} ${m.kind === 'pack' ? 'jogos' : 'dias'}`, tone: 'var(--text2)' };
  }
  // A finished or lost period has nothing left to count down.
  const left = m.locked || m.failed || m.done ? null : daysLeft(m.resetDate);
  if (!outcome && !left) return null;

  const figure = m.locked || m.failed ? `+${money(m.points)}` : m.claimed != null && m.claimed > 0 ? money(m.claimed) : null;
  return h(
    'div',
    { class: 'mstatus' },
    left && [icon('clock'), h('span', null, left)],
    h('span', { class: 'sp' }),
    outcome && [
      h('span', { class: 'ellip', style: { color: outcome.tone } }, outcome.text),
      outcome.glyph === 'x' ? h('span', { style: { color: outcome.tone, fontWeight: 600 } }, '✕') : outcome.glyph && h('span', { style: { color: outcome.tone, display: 'inline-flex' } }, icon(outcome.glyph)),
      figure && h('span', { class: 'fig' }, figure),
    ],
  );
}
