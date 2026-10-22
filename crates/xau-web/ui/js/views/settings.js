// Ajustes: the notices, the one-account-at-a-time rule, and what this
// server is. Every switch applies at once; there is no "save".

import { h } from '../dom.js';
import { S, prefs } from '../state.js';
import { header, switchEl } from '../ui.js';
import * as act from '../actions.js';
import { supported, permission, ask } from '../notify.js';

export function mount(root) {
  const page = h('div', { class: 'page' });
  root.append(page);

  function render() {
    page.replaceChildren(
      header({ back: '#/', title: 'Ajustes' }),
      h(
        'div',
        { class: 'stack' },
        group('Notificações', [
          permissionNotice(),
          toggle('Avisar quando o farm terminar', 'Uma notificação quando uma conta completa os 15 minutos nos 8 jogos.', prefs.get('notifyDone', true), (on) => {
            prefs.set('notifyDone', on);
            render();
          }),
          toggle('Avisar quando subir de nível', 'Uma notificação assim que uma conta sobe de nível no Rewards (visto a cada leitura do saldo).', prefs.get('notifyLevel', true), (on) => {
            prefs.set('notifyLevel', on);
            render();
          }),
        ]),
        group('Contas', [
          toggle('Uma conta ativa por vez', 'Só uma conta fica descongelada. Descongelar outra congela as demais na hora e para o farm delas.', S.singleActive, (on) => act.setSingleActive(on)),
        ]),
        group('Sobre', [
          info('Versão', S.info?.version ?? '—'),
          info('Sistema', { windows: 'Windows', linux: 'Linux', macos: 'macOS' }[S.info?.os] ?? S.info?.os ?? '—'),
        ]),
      ),
    );
  }

  /** Browser notifications need the browser's permission; say so, once it matters. */
  function permissionNotice() {
    if (!supported) return h('div', { class: 'blocked' }, 'Este navegador não mostra notificações; os avisos aparecem aqui na página.');
    const p = permission();
    if (p === 'granted') return null;
    return h(
      'div',
      { class: 'blocked' },
      p === 'denied'
        ? 'As notificações deste site estão bloqueadas no navegador: os avisos aparecem só aqui na página. Para receber fora dela, libere nas configurações do site.'
        : 'Os avisos aparecem aqui na página. Para recebê-los também com a aba em segundo plano, permita as notificações.',
      p === 'default' &&
        h('div', { style: { marginTop: '8px' } }, h('button', { class: 'btn', onClick: async () => { await ask(); render(); } }, 'Permitir notificações')),
    );
  }

  render();
  return { update: render, destroy() {} };
}

const group = (label, rows) => h('div', null, h('div', { class: 'group-label' }, label), h('div', { class: 'group' }, rows.filter(Boolean)));

function toggle(title, detail, checked, onChange) {
  return h(
    'div',
    { class: 'toggle', role: 'group' },
    h('div', { class: 'grow' }, h('div', { class: 't' }, title), h('div', { class: 'd' }, detail)),
    switchEl(checked, onChange, title),
  );
}

const info = (k, v) => h('div', { class: 'info' }, h('span', { class: 't2' }, k), h('span', { class: 'v' }, v));
