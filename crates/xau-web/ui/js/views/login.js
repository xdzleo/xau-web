// Sign-in, laid out like the app's LoginScreen: a full screen with the title
// row and "Cancelar", and Microsoft's own page under it. The page runs in a
// real browser engine on the computer the server runs on (it needs the
// core's script, which a normal page cannot host) and is drawn here; what is
// clicked and typed here is replayed there. The core decides what every
// message from the page means, exactly as on Android.

import { h, put, icon, sleep } from '../dom.js';
import { api } from '../api.js';
import { S } from '../state.js';
import { toast, skeleton, stateBlock } from '../ui.js';
import { refresh } from '../actions.js';
import * as sj from '../sjlogin.js';

/**
 * Sign in. The page runs in this browser when it can (sjlogin.js: instant typing, the browser's own
 * keyboard and password manager); when it cannot, or it does not open in time, in a browser on the
 * server drawn here (`startStreamLogin`, below).
 */
export async function startLogin() {
  if (sj.wanted()) {
    try {
      while ((await sj.startDirect()) === 'retry');
      return;
    } catch (e) {
      if (!(e instanceof sj.Fallback)) throw e;
      console.warn('login no navegador:', e.message);
    }
  }
  return startStreamLogin();
}

async function startStreamLogin() {
  let id = null;
  let open = true;
  let pageView = null;

  const stage = h('div', { class: 'loginstage' });
  const cancelBtn = h('button', { class: 'ghostbtn', onClick: () => close() }, 'Cancelar');
  const root = h(
    'div',
    { class: 'loginscreen', role: 'dialog', 'aria-label': 'Entrar na conta' },
    h('div', { class: 'loginbar' }, icon('xbox', 'loginic'), h('span', { class: 'row-title' }, 'Entrar na conta'), h('span', { class: 'grow' }), cancelBtn),
    stage,
  );
  document.body.append(root);
  document.documentElement.classList.add('logging');

  function close() {
    if (!open) return;
    open = false;
    pageView?.stop();
    pageView = null;
    root.remove();
    document.documentElement.classList.remove('logging');
    if (id) api.loginCancel(id).catch(() => {});
  }
  const show = (...kids) => put(stage, ...kids);
  const failed = (title, detail, actionLabel, onAction) => {
    id = null;
    pageView?.stop();
    pageView = null;
    show(stateBlock({ title, detail, actionLabel, onAction, tone: 'danger' }));
  };

  if (S.info && !S.info.signIn) {
    failed('Não foi possível abrir o login', 'Este servidor não consegue abrir o navegador do login (falta o Chromium ou o WebView2).', 'Voltar', close);
    return;
  }

  // The first sign-in also registers this install as a Windows device with Microsoft, a few seconds more.
  show(h('div', { class: 'loginwait' }, h('p', { class: 'body t2' }, 'Preparando o login do Windows…'), skeleton(220, 24), skeleton(320, 15), skeleton(280, 15)));
  try {
    id = await api.loginStart();
  } catch (e) {
    failed('Não foi possível abrir o login', e.message, 'Voltar', close);
    return;
  }

  let shown = '';
  while (open) {
    let st;
    try {
      st = await api.loginStatus(id);
    } catch (e) {
      failed('Não foi possível acompanhar o login', e.message, 'Voltar', close);
      return;
    }
    if (st.state === 'waiting') {
      // Drawn once: the page keeps its own state (what was typed, which step).
      if (shown !== 'page') {
        shown = 'page';
        pageView = startPageView(id);
        show(pageView.el);
      }
    } else if (st.state === 'working') {
      if (shown !== 'working') {
        shown = 'working';
        pageView?.stop();
        pageView = null;
        show(h('div', { class: 'loginwork' }, h('div', { class: 'section' }, 'Conectando à Xbox Live'), h('p', { class: 'body t2' }, 'Validando o login do Windows e lendo o perfil.')));
      }
    } else if (st.state === 'done') {
      id = null; // done: nothing left to cancel
      close();
      toast(st.warning || 'Conta conectada', st.warning ? 'err' : '');
      await refresh();
      return;
    } else if (st.state === 'failed') {
      failed('Não foi possível entrar', st.error || 'o login foi recusado', 'Tentar de novo', () => {
        close();
        startStreamLogin();
      });
      return;
    } else {
      // cancelled: nothing to say
      id = null;
      close();
      return;
    }
    await sleep(1000);
  }
}
// ------------------------------------------------------- the page, here

const VK = { Backspace: 8, Tab: 9, Enter: 13, Escape: 27, End: 35, Home: 36, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Delete: 46, PageUp: 33, PageDown: 34 };

/**
 * Microsoft's sign-in page, drawn here. The server runs it (it needs the
 * core's script, which a normal page cannot host) at exactly the size of
 * this screen. One WebSocket carries both ways: the engine's frames in (one
 * each time the page draws; only the newest is ever drawn) and the person's
 * touches, clicks and keys out, replayed there as real input.
 */
export function startPageView(id) {
  let alive = true;
  let editing = false;
  let ws = null;
  let polling = false; // fallback: one picture at a time over HTTP

  const canvas = h('canvas', { class: 'pageimg', 'aria-label': 'Página de login da Microsoft', role: 'img' });
  const ctx = canvas.getContext('2d', { alpha: false, desynchronized: true });
  // A real field, only to hold the keyboard (and bring up the phone's).
  const kbd = h('input', { class: 'pagekbd', type: 'text', autocomplete: 'off', autocorrect: 'off', autocapitalize: 'off', spellcheck: 'false', enterkeyhint: 'go', 'aria-label': 'Teclado da página de login' });
  const el = h('div', { class: 'pageview', tabindex: '-1' }, canvas, kbd, h('div', { class: 'pageload' }, 'Carregando a página da Microsoft…'));

  const dpr = () => Math.min(2, window.devicePixelRatio || 1);
  const size = () => `w=${Math.round(el.clientWidth)}&h=${Math.round(el.clientHeight)}&d=${Math.round(dpr() * 100)}`;

  // Newest frame wins: while one decodes, later ones replace each other and only the last is drawn.
  let decoding = false;
  let pending = null;
  const draw = async (blob) => {
    if (decoding) {
      pending = blob;
      return;
    }
    decoding = true;
    try {
      const bmp = await createImageBitmap(blob);
      // The canvas is this screen in device pixels; fast frames (CSS pixels) and sharp ones
      // (device pixels) are both drawn to fill it, so it never changes size between them.
      const w = Math.round(el.clientWidth * dpr()) || bmp.width;
      const hh = Math.round(el.clientHeight * dpr()) || bmp.height;
      if (canvas.width !== w || canvas.height !== hh) {
        canvas.width = w;
        canvas.height = hh;
        ctx.imageSmoothingQuality = 'high';
      }
      ctx.drawImage(bmp, 0, 0, w, hh);
      bmp.close();
      el.classList.add('live');
    } catch {
      // a broken frame: the next one replaces it
    }
    decoding = false;
    if (pending) {
      const next = pending;
      pending = null;
      draw(next);
    }
  };

  // The page answers every input with whether a field has the focus. Right after a tap, the
  // answer to the touch's start (before the field took it) can still be on its way: a "no" that
  // close to the tap waits for the later answers before the keyboard goes away.
  let tappedAt = 0;
  let letGo = 0;
  const keyboard = (on) => {
    editing = on;
    clearTimeout(letGo);
    if (on) kbd.focus({ preventScroll: true });
    else if (document.activeElement === kbd) {
      letGo = setTimeout(() => {
        if (!editing && document.activeElement === kbd) kbd.blur();
      }, Math.max(0, tappedAt + 450 - Date.now()));
    }
  };

  let shown = '';
  let gotFrame = false;
  const connect = () => {
    if (!alive || polling) return;
    // Not laid out yet (a tab opened in the background, a screen still sliding in): the page
    // must get this screen's real size, or it is drawn tiny and stretched.
    if (el.clientWidth < 50 || el.clientHeight < 50) {
      clearTimeout(connect.t);
      connect.t = setTimeout(connect, 100);
      return;
    }
    const s = size();
    if (s === shown && ws && ws.readyState <= 1) return;
    shown = s;
    ws?.close();
    const sock = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/login/${id}/ws?${s}`);
    sock.binaryType = 'blob';
    sock.onmessage = (m) => {
      if (typeof m.data === 'string') {
        try {
          const j = JSON.parse(m.data);
          if (typeof j.editing === 'boolean') keyboard(j.editing);
        } catch {}
        return;
      }
      gotFrame = true;
      draw(m.data);
    };
    sock.onclose = () => {
      if (ws !== sock || !alive) return;
      ws = null;
      if (!gotFrame) {
        polling = true; // the live picture never came: fall back to pictures
        pollPictures();
      } else setTimeout(connect, 300); // dropped: reconnect (the sign-in may simply have ended)
    };
    ws = sock;
  };
  const resized = new ResizeObserver(() => {
    clearTimeout(resized.t);
    resized.t = setTimeout(connect, 120);
  });
  resized.observe(el);
  setTimeout(connect, 0); // once the view is in the page
  // The size is re-checked now and then too: a resize the observer misses (a tab coming back
  // from the background) must not leave the page at the wrong size.
  const resizeCheck = setInterval(() => {
    if (el.clientWidth >= 50 && size() !== shown) connect();
  }, 400);
  window.addEventListener('resize', connect);

  const send = (ev) => {
    if (ws && ws.readyState === 1) {
      ws.send(JSON.stringify(ev));
      return;
    }
    fetch(`/api/login/${id}/input`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-xau': '1' },
      body: JSON.stringify(ev),
      credentials: 'same-origin',
      keepalive: true,
    })
      .then((r) => r.json())
      .then((j) => {
        if (j && j.ok && typeof j.ok.editing === 'boolean') keyboard(j.ok.editing);
      })
      .catch(() => {});
  };
  // The page is exactly this big, so a point here is a point there.
  const at = (e) => {
    const r = canvas.getBoundingClientRect();
    return { x: Math.round(e.clientX - r.left), y: Math.round(e.clientY - r.top) };
  };
  const btn = (e) => (e.button === 2 ? 'right' : e.button === 1 ? 'middle' : 'left');

  // Moves are coalesced to one per display frame: never queued behind each other.
  let pendingMove = null;
  let moveScheduled = false;
  const flushMove = () => {
    moveScheduled = false;
    if (pendingMove) send(pendingMove);
    pendingMove = null;
  };
  const queueMove = (ev) => {
    pendingMove = ev;
    if (!moveScheduled) {
      moveScheduled = true;
      requestAnimationFrame(flushMove);
    }
  };

  let touching = false;
  canvas.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    canvas.setPointerCapture?.(e.pointerId);
    if (e.pointerType === 'mouse') send({ kind: 'mouse', type: 'mousePressed', ...at(e), button: btn(e), clickCount: 1 });
    else {
      touching = true;
      send({ kind: 'touch', type: 'touchStart', ...at(e) });
    }
  });
  canvas.addEventListener('pointermove', (e) => {
    if (e.pointerType === 'mouse') queueMove({ kind: 'mouse', type: 'mouseMoved', ...at(e), button: e.buttons & 1 ? 'left' : 'none' });
    else if (touching) queueMove({ kind: 'touch', type: 'touchMove', ...at(e) });
  });
  const up = (e) => {
    e.preventDefault();
    if (pendingMove) flushMove();
    if (e.pointerType === 'mouse') send({ kind: 'mouse', type: 'mouseReleased', ...at(e), button: btn(e), clickCount: 1 });
    else if (touching) {
      touching = false;
      send({ kind: 'touch', type: e.type === 'pointercancel' ? 'touchCancel' : 'touchEnd', ...at(e) });
      // A phone opens its keyboard only for a field focused during the tap itself; the page says
      // whether the tap landed in a field only afterwards. So the field takes focus now, and is
      // let go again if the page says it was not a text field.
      if (e.type === 'pointerup') {
        tappedAt = Date.now();
        kbd.focus({ preventScroll: true });
      }
    }
  };
  canvas.addEventListener('pointerup', up);
  canvas.addEventListener('pointercancel', up);
  canvas.addEventListener('contextmenu', (e) => e.preventDefault());
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    send({ kind: 'mouse', type: 'mouseWheel', ...at(e), deltaX: e.deltaX, deltaY: e.deltaY });
  }, { passive: false });

  // Text (typed, pasted, or from a phone's keyboard) arrives as input events; keys that are
  // not text go as key events.
  //
  // A phone keyboard edits the field as a whole: it composes a word in place, swaps it for a
  // correction, and deletes with no key event at all. So the field is mirrored: whatever changed
  // since the last look goes to the page as Backspaces plus the new text. A few padding
  // characters sit in front, so a Backspace in a field that looks empty still deletes something
  // (and reaches the page).
  const PAD = ' '.repeat(8);
  let mirrored = PAD;
  let composing = false;
  const resetField = () => {
    kbd.value = PAD;
    mirrored = PAD;
    try { kbd.setSelectionRange(PAD.length, PAD.length); } catch {}
  };
  const backspace = () => {
    send({ kind: 'key', type: 'rawKeyDown', key: 'Backspace', code: 'Backspace', vk: 8, modifiers: 0 });
    send({ kind: 'key', type: 'keyUp', key: 'Backspace', code: 'Backspace', vk: 8, modifiers: 0 });
  };
  const sync = () => {
    const now = kbd.value;
    let same = 0;
    while (same < now.length && same < mirrored.length && now[same] === mirrored[same]) same++;
    for (let i = same; i < mirrored.length; i++) backspace();
    const added = now.slice(same).replace(/ /g, ' ');
    if (added) send({ kind: 'text', text: added });
    mirrored = now;
    // Back to the padding between words, never in the middle of one being composed.
    if (!composing && (now.length < PAD.length || now.length > PAD.length + 40 || !now.startsWith(PAD))) resetField();
  };
  kbd.addEventListener('compositionstart', () => { composing = true; });
  kbd.addEventListener('compositionend', () => { composing = false; sync(); });
  kbd.addEventListener('input', sync);
  kbd.addEventListener('focus', resetField);
  resetField();
  const keyEv = (type) => (e) => {
    const vk = VK[e.key];
    if (vk == null || e.ctrlKey || e.metaKey) return;
    // A phone keyboard's Backspace edits the field (mirrored above), whatever key event it sends.
    if (e.key === 'Backspace' && !matchMedia('(pointer: fine)').matches) return;
    e.preventDefault();
    e.stopPropagation(); // Esc belongs to the Microsoft page here; "Cancelar" closes the screen
    const modifiers = (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
    send({ kind: 'key', type: type === 'keyDown' ? 'rawKeyDown' : 'keyUp', key: e.key, code: e.code, vk, modifiers, text: type === 'keyDown' && e.key === 'Enter' ? '\r' : undefined });
  };
  kbd.addEventListener('keydown', keyEv('keyDown'));
  kbd.addEventListener('keyup', keyEv('keyUp'));
  // On a computer, typing goes to the page without clicking a field first.
  if (matchMedia('(pointer: fine)').matches) kbd.focus({ preventScroll: true });

  async function pollPictures() {
    let gone = 0;
    while (alive) {
      try {
        const r = await fetch(`/api/login/${id}/frame?${size()}`, { credentials: 'same-origin' });
        if (r.ok) {
          gone = 0;
          editing = r.headers.get('x-edit') === '1';
          await draw(await r.blob());
        } else if (r.status === 410 && ++gone > 3) break;
      } catch {
        // a dropped picture is not worth a message
      }
      await sleep(16);
    }
  }

  return {
    el,
    send, // also used by the end-to-end benchmark
    stop() {
      alive = false;
      resized.disconnect();
      clearInterval(resizeCheck);
      clearTimeout(connect.t);
      window.removeEventListener('resize', connect);
      ws?.close();
      ws = null;
    },
  };
}