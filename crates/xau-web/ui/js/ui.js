// Pieces every screen uses: toasts, dialogs, the back header, empty states.

import { h, icon, img } from './dom.js';

export function toast(text, kind = '') {
  const el = h('div', { class: `toast ${kind}`, role: 'status' }, text);
  document.getElementById('toasts').append(el);
  setTimeout(() => el.remove(), kind === 'err' ? 6000 : 3800);
}

/** Show `content` over the page. Esc and a click outside close it. */
export function modal(content, { bottom = false, onClose } = {}) {
  const backdrop = h('div', { class: `backdrop${bottom ? ' bottom' : ''}` }, content);
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    document.removeEventListener('keydown', onKey);
    backdrop.remove();
    onClose?.();
  };
  const onKey = (e) => e.key === 'Escape' && close();
  backdrop.addEventListener('mousedown', (e) => e.target === backdrop && close());
  document.addEventListener('keydown', onKey);
  document.body.append(backdrop);
  return { close, el: backdrop };
}

/** A yes/no question. Resolves true only on the confirm button. */
export function confirmDialog({ title, text, confirm = 'Confirmar', cancel = 'Cancelar', danger = false }) {
  return new Promise((resolve) => {
    let answer = false;
    const m = modal(
      h(
        'div',
        { class: 'dialog', role: 'alertdialog', 'aria-label': title },
        h('h3', null, title),
        h('p', null, text),
        h(
          'div',
          { class: 'actions' },
          h('button', { class: 'btn ghost', onClick: () => m.close() }, cancel),
          h(
            'button',
            {
              class: `btn ${danger ? 'danger' : 'primary'}`,
              onClick: () => {
                answer = true;
                m.close();
              },
            },
            confirm,
          ),
        ),
      ),
      { onClose: () => resolve(answer) },
    );
  });
}

export const skeleton = (w, hgt) => h('div', { class: 'skeleton', style: { width: `${w}px`, maxWidth: '100%', height: `${hgt}px` } });

/** The empty / error panel, with an optional action. */
export function stateBlock({ glyph, title, detail, actionLabel, onAction, tone }) {
  return h(
    'div',
    { class: 'state' },
    glyph && h('div', { class: 'big' }, glyph),
    h('h3', { class: tone === 'danger' ? 'danger' : '' }, title),
    detail && h('p', null, detail),
    actionLabel && h('button', { class: 'btn primary', onClick: onAction }, actionLabel),
  );
}

/** The back button and title row shared by every inner screen. */
export function header({ back, title, sub, lead, right }) {
  return h(
    'div',
    { class: 'header' },
    h('a', { class: 'back', href: back, 'aria-label': 'Voltar' }, '‹'),
    lead,
    h('div', { class: 'grow' }, h('div', { class: 'section ellip' }, title), sub && h('div', { class: 'caption t2 ellip' }, sub)),
    right,
  );
}

/** A game's art, or its first letters when Xbox had none. */
export function gameArt(t, cls = '') {
  const el = h('div', { class: `art ${cls}` });
  const pic = img(t.image, t.name);
  if (pic) el.append(pic);
  else el.append((t.name || '?').slice(0, 2).toUpperCase());
  return el;
}

export function avatar(account, cls = '') {
  const el = h('div', { class: `avatar ${cls}` });
  const pic = img(account.gamerpic, '');
  if (pic) el.append(pic);
  else el.append((account.gamertag || '?').slice(0, 1).toUpperCase());
  return el;
}

export const switchEl = (checked, onChange, label) =>
  h('button', {
    class: 'switch',
    role: 'switch',
    'aria-checked': String(!!checked),
    'aria-label': label,
    onClick: (e) => {
      e.stopPropagation();
      onChange(!checked);
    },
  });

export { icon };
