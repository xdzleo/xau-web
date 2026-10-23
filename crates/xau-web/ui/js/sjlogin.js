// Microsoft's sign-in page, run in the person's own browser.
//
// The other way to sign in (login.js, `startStreamLogin`) runs the page in a browser on the
// server and sends its picture here: every letter typed goes to the server and back as a picture.
// This way the page runs *here*, as a real page of this browser, so the keyboard, the
// password manager, scrolling and every tap are the browser's own and instant. What lets a page
// of another site run inside this one is Scramjet (a service worker that rewrites the page and
// its requests) with a libcurl transport that does the TLS in WebAssembly; the bytes travel to
// Microsoft over one WebSocket to this site's `/wisp/` and leave the server only through
// Cloudflare (see `src/wisp.rs`). The server's core still does everything that is secret: it
// registers the Windows device, reads the sign-in's token and keeps the account.
//
// The page is loaded ahead of time, out of sight, while the person is still on the welcome
// screen, so "Entrar" shows a page that is already there. Nothing is registered with Microsoft
// until they press it (the device is registered then, in parallel with their typing).
//
// Anything that does not work (no service worker, a slow or stuck page) ends in `Fallback`, and
// login.js uses the other way.

import { h, icon } from './dom.js';
import { api } from './api.js';
import { S } from './state.js';
import { toast, skeleton, stateBlock } from './ui.js';

export class Fallback extends Error {}

/** Microsoft's own hosts: what gets the Windows web host's identity. */
const MS = /(^|\.)(live\.com|microsoft\.com|microsoftonline\.com|msftauth\.net|msauth\.net|xboxlive\.com|xbox\.com|gfx\.ms)$/;
const INLINE_LOGIN = 'https://login.live.com/ppsecure/InlineLogin.srf';
/** Microsoft's sign-up runs as an ordinary phone page (the Windows-hosted one waits forever). */
const SIGNUP_UA = 'Mozilla/5.0 (Linux; Android 15; moto g34 5G) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Mobile Safari/537.36';
const LANGUAGES = 'pt-BR,en-US;q=0.9';

const TIMEOUTS = window.__sjTimeouts || {}; // (the tests shorten these)
const RUNTIME_TIMEOUT = TIMEOUTS.runtime ?? 25_000; // the runtime (scripts, service worker, transport) must be up by then
const FIELD_TIMEOUT = TIMEOUTS.field ?? 25_000; // the page must show its field by then, counted from when it is wanted
const PAGE_LIFETIME = 4 * 60_000; // a page loaded ahead and left this long is loaded again
const KEEP_PRELOADED = 10 * 60_000; // ...and one nobody asked for in this long is dropped

// Marks of the work, for measuring (window.__sj in the console).
const dbg = (window.__sj = { t0: performance.now(), marks: [], entry: null });
const mark = (name) => dbg.marks.push([name, Math.round(performance.now() - dbg.t0)]);

const withTimeout = (p, ms, what) =>
  new Promise((ok, no) => {
    const t = setTimeout(() => no(new Fallback(`${what}: sem resposta em ${Math.round(ms / 1000)} s`)), ms);
    p.then(
      (v) => (clearTimeout(t), ok(v)),
      (e) => (clearTimeout(t), no(e)),
    );
  });

export function supported() {
  try {
    return !!(
      window.isSecureContext &&
      navigator.serviceWorker &&
      typeof WebAssembly === 'object' &&
      window.BroadcastChannel &&
      window.MessageChannel &&
      window.indexedDB &&
      window.ReadableStream &&
      window.fetch
    );
  } catch {
    return false;
  }
}

/** Whether to sign in this way: the server has the files, this browser can run them, and the person did not ask otherwise (`?login=stream`). */
export function wanted() {
  let forced = new URLSearchParams(location.search).get('login');
  if (!forced) {
    try {
      forced = localStorage.getItem('xau.login');
    } catch {}
  }
  if (forced === 'stream') return false;
  return supported() && !!S.info?.scramjet;
}

// ------------------------------------------------------------------ the runtime

let runtimeP = null;

function loadScript(src) {
  return new Promise((ok, no) => {
    const s = document.createElement('script');
    s.src = src;
    s.async = false;
    s.onload = () => ok();
    s.onerror = () => no(new Error(`não foi possível carregar ${src}`));
    document.head.append(s);
  });
}

function preloadFile(href, as) {
  const l = document.createElement('link');
  l.rel = 'preload';
  l.as = as;
  l.href = href;
  if (as === 'fetch') l.crossOrigin = 'anonymous';
  document.head.append(l);
}

async function registerWorker() {
  const reg = await navigator.serviceWorker.register('/sw.js', { type: 'classic', updateViaCache: 'none' });
  await navigator.serviceWorker.ready;
  if (reg.active) return reg.active;
  const w = reg.installing || reg.waiting;
  if (!w) throw new Error('o service worker não ficou ativo');
  await new Promise((ok) => {
    if (w.state === 'activated') return ok();
    w.addEventListener('statechange', () => w.state === 'activated' && ok());
  });
  return reg.active || w;
}

/** The runtime, loaded once: the scripts all at once (the browser has the first three on the way before
 *  they are asked for), the service worker beside them, then the controller over the transport. */
function loadRuntime() {
  if (runtimeP) return runtimeP;
  const base = `/sj/${S.info.scramjet}`;
  const file = (p) => `${base}/${p}`;
  runtimeP = (async () => {
    mark('runtime:start');
    for (const [p, as] of [['scram/scramjet.js', 'script'], ['controller/controller.api.js', 'script'], ['clients/libcurl-client.js', 'script'], ['scram/scramjet.wasm', 'fetch']]) {
      preloadFile(file(p), as);
    }
    const worker = registerWorker();
    await loadScript(file('scram/scramjet.js'));
    await loadScript(file('controller/controller.api.js'));
    await loadScript(file('clients/libcurl-client.js'));
    mark('runtime:scripts');
    const sw = await worker;
    mark('runtime:worker');
    const { Controller, config, ManagedPlugin } = window.$scramjetController;
    config.injectPath = file('controller/controller.inject.js');
    config.wasmPath = file('scram/scramjet.wasm');
    config.scramjetPath = file('scram/scramjet.js');
    const wisp = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/wisp/`;
    const transport = new window.LibcurlTransport.LibcurlClient({ wisp });
    const controller = new Controller({ serviceworker: sw, transport });
    warmUp(transport).catch(() => {});
    await controller.wait();
    mark('runtime:ready');
    return { controller, ManagedPlugin, ScramjetHeaders: window.$scramjet?.ScramjetHeaders };
  })();
  runtimeP.catch(() => {
    runtimeP = null;
  });
  return runtimeP;
}

/** Open the connection to the host the sign-in page's files come from (TCP and TLS through the server: a few
 *  round trips) while the page itself is still being fetched, so its files find it open. Only that host: the
 *  page's own address is asked for at the same moment, and two requests to one host that are not yet
 *  connected make two connections (measured: slower). Nothing of the person's is sent: an empty request for a
 *  file that is not there. */
async function warmUp(transport) {
  // The transport has no session until its WebAssembly is up (the controller starts that; it is not
  // ours to start twice), so wait for it, not for long.
  for (let i = 0; i < 150 && !transport.session; i++) await new Promise((r) => setTimeout(r, 20));
  if (!transport.session) return;
  mark('runtime:warming');
  for (const host of ['logincdn.msftauth.net']) {
    try {
      Promise.resolve(transport.request(new URL(`https://${host}/favicon.ico`), 'GET', null, [], undefined))
        .then((r) => {
          mark(`warm:${host}`);
          return r?.body?.cancel?.();
        })
        .catch(() => {});
    } catch {}
  }
}

/** Forget what Microsoft's pages stored here (cookies; the controller keeps them in this browser): one
 *  sign-in must never see the one before it, and none of it should outlive the sign-in. */
async function wipeState(controller) {
  try {
    controller.cookieJar.clear();
  } catch {}
  try {
    await new Promise((ok) => {
      const open = indexedDB.open('__scramjet_controller', 1);
      open.onupgradeneeded = () => {
        if (!open.result.objectStoreNames.contains('state')) open.result.createObjectStore('state');
      };
      open.onerror = () => ok();
      open.onsuccess = () => {
        try {
          const t = open.result.transaction('state', 'readwrite');
          t.objectStore('state').clear();
          t.oncomplete = t.onerror = t.onabort = () => (open.result.close(), ok());
        } catch {
          ok();
        }
      };
    });
  } catch {}
}

// -------------------------------------------------------------------- the page

/** Set a request header in whatever shape the transport's request has it (Scramjet: a list of pairs). */
function setHeader(init, name, value) {
  const hs = init.headers;
  const lower = name.toLowerCase();
  if (Array.isArray(hs)) {
    const i = hs.findIndex(([k]) => String(k).toLowerCase() === lower);
    if (i >= 0) hs[i] = [name, value];
    else hs.push([name, value]);
  } else if (typeof Headers !== 'undefined' && hs instanceof Headers) hs.set(name, value);
  else if (hs && typeof hs === 'object') {
    for (const k of Object.keys(hs)) if (k.toLowerCase() === lower) delete hs[k];
    hs[name] = value;
  } else init.headers = [[name, value]];
}

/** Microsoft's sign-up link carries the Windows host's marks, and the page then waits for the host forever. */
function unhostedSignup(href) {
  // Text, not URLSearchParams: what stays must stay byte for byte (the same as `unhosted_signup` in webhost.rs).
  const q = href.indexOf('?');
  if (q < 0 || !href.startsWith('https://signup.live.com/')) return null;
  const base = href.slice(0, q);
  const hosted = (p) => {
    const k = p.split('=')[0].toLowerCase();
    return k === 'hosted' || k === 'platform' || k === 'uiflavor' || k.startsWith('cxh-');
  };
  const parts = href.slice(q + 1).split('&');
  if (!parts.some(hosted)) return null;
  const kept = parts.filter((p) => !hosted(p));
  return kept.length ? `${base}?${kept.join('&')}` : base;
}

/** The page that started a navigation, as Microsoft's page sees its address (the service worker names it by its
 *  own address, `/~/sj/<frame>/<page>/<address, encoded>?<extras>`); null when it was not a page of the sign-in. */
function initiatorOf(request) {
  const u = String(request.rawClientUrl || '');
  const i = u.indexOf('/~/sj/');
  if (i < 0) return null;
  try {
    return decodeURIComponent(u.slice(i + 6).split('/').slice(2).join('/').split('?')[0]).split('#')[0];
  } catch {
    return '';
  }
}

/** The core asks the page for its host context with this message; the same test as `PageMessage::ContextRequest`. */
function contextRequest(body) {
  try {
    const j = JSON.parse(body);
    if (j?.type === 'invoke' && j?.value?.name === 'CloudExperienceHost.getContext' && typeof j.value.context === 'string') return j.value.context;
  } catch {}
  return null;
}

function pluginClass(Base) {
  return class XauLogin extends Base {
    constructor(page) {
      super('xau-login', []);
      this.page = page;
    }

    install(frame) {
      super.install(frame);
      const page = this.page;
      const isNavigation = (c) => ['document', 'iframe'].includes(c.parsed.destination);
      // A navigation of the sign-in's own window (not of a frame inside the page): this site started it, or the page on show did.
      const isTopNavigation = (c) => {
        const from = initiatorOf(c.request);
        return from === null || from === page.topHref();
      };

      // The sign-up link, and the way back from it.
      this.tap(frame.hooks.fetch.intercept, (c, p) => {
        if (!isNavigation(c)) return;
        const url = c.parsed.url;
        const clean = unhostedSignup(url.href);
        window.__sjTrace?.('navigation', url.href.slice(0, 100), 'clean:', clean && clean.slice(0, 100), 'top:', isTopNavigation(c));
        if (clean) {
          // The page the link opens waits for the Windows host forever; the same page without its marks does not.
          page.inSignup = true;
          p.response = page.nothing();
          queueMicrotask(() => frame.go(clean));
        } else if (page.inSignup && url.href.startsWith(INLINE_LOGIN) && isTopNavigation(c)) {
          // The sign-up sends the person back to the sign-in (done, or "Sign in"): back to the sign-in, as it starts.
          page.inSignup = false;
          p.response = page.nothing();
          queueMicrotask(() => page.load());
        }
      });

      // Every request to Microsoft is the Windows web host's; the first one for the sign-in page carries its marks.
      this.tap(frame.hooks.fetch.request, (c, p) => {
        const host = p.url.hostname;
        if (!MS.test(host)) return;
        const signup = host === 'signup.live.com';
        window.__sjTrace?.('request', p.url.href.slice(0, 100));
        setHeader(p.init, 'User-Agent', signup ? SIGNUP_UA : page.spec.user_agent);
        setHeader(p.init, 'Accept-Language', LANGUAGES);
        if (!signup && page.marks && ['document', 'iframe'].includes(c.parsed.destination) && p.url.pathname.toLowerCase() === '/ppsecure/inlinelogin.srf') {
          page.marks = false;
          for (const [k, v] of page.spec.headers) setHeader(p.init, k, v);
          mark('page:request-with-marks');
        }
      });

      // Each page gets the host's bridge before its own scripts run.
      this.tap(frame.hooks.init.post, (c) => {
        if (!c.isTopLevel) return;
        const w = c.window;
        page.win = w;
        page.client = c.client;
        const signup = String(c.client?.url?.hostname || '') === 'signup.live.com';
        try {
          if (!signup) {
            Object.defineProperty(w.navigator, 'userAgent', { get: () => page.spec.user_agent, configurable: true });
            Object.defineProperty(w.navigator, 'platform', { get: () => 'Linux aarch64', configurable: true });
          }
        } catch {}
        const bridge = { postMessage: (m) => page.fromPage(String(m)) };
        try {
          Object.defineProperty(w, page.spec.bridge, { value: bridge, configurable: true, writable: true });
          Object.defineProperty(w, 'external', { value: { notify: (m) => bridge.postMessage(m) }, configurable: true, writable: true });
        } catch (e) {
          console.warn('xau login: bridge', e);
        }
        w.addEventListener('load', () => page.loaded(String(c.client?.url?.href || '')));
        try {
          window.__sjHook?.(w, c); // for looking inside the page while testing
        } catch {}
      });
    }
  };
}

class Page {
  constructor(rt, spec, screen) {
    this.rt = rt;
    this.spec = spec;
    this.screen = screen;
    this.caps = (spec.headers.find(([k]) => k.toLowerCase() === 'cxh-capabilities') || [])[1] || '';
    this.win = null;
    this.marks = true; // the next sign-in page request carries the Windows host's marks
    this.inSignup = false;
    this.ready = false;
    this.loadedAt = 0;
    this.on = {}; // message, ready, finish
    this.poll = 0;
    this.frame = rt.controller.createFrame(screen.iframe, { plugins: [new rt.Plugin(this)] });
  }

  /** The address of the page on show, as it sees it. */
  topHref() {
    return String(this.client?.url?.href || '').split('#')[0];
  }

  /** A response that makes the browser stay where it is (a navigation the plugin has redirected itself). */
  nothing() {
    const headers = this.rt.ScramjetHeaders ? new this.rt.ScramjetHeaders() : undefined;
    return { body: '', headers, status: 204, statusText: 'No Content' };
  }

  load() {
    this.marks = true;
    this.ready = false;
    this.loadedAt = Date.now();
    mark('page:go');
    clearInterval(this.poll);
    this.poll = setInterval(() => this.look(), 120);
    this.frame.go(this.spec.url);
  }

  /** Whether the page shows something to type into. */
  look() {
    try {
      const d = this.screen.iframe.contentDocument;
      if (!d || !d.body) return;
      const field = d.querySelector('input:not([type=hidden]):not([type=submit]):not([type=button]):not([type=checkbox]):not([type=radio])');
      if (field && (d.body.innerText || '').trim().length > 10) {
        this.ready = true;
        clearInterval(this.poll);
        mark('page:ready');
        this.on.ready?.();
      }
    } catch {}
  }

  /** A message the page posted to its host. The context question is answered here, at once; the rest is the core's. */
  fromPage(body) {
    const ctx = contextRequest(body);
    if (ctx === null) return this.on.message?.(body);
    const reply = JSON.stringify({ type: 'callback', value: { name: 'CloudExperienceHost.getContext', args: ['CloudExperienceHost', 'TokenBroker', 'TokenBroker', this.caps], context: ctx } });
    try {
      this.win['CloudExperienceHost.Bridge.dispatchMessage'](reply);
    } catch (e) {
      console.warn('xau login: context', e);
    }
  }

  /** The page finished loading: on the sign-in's last page, hand its data to the core. */
  loaded(href) {
    mark(`page:loaded ${href.slice(0, 60)}`);
    if (this.spec.finish_url_prefix && href.startsWith(this.spec.finish_url_prefix)) this.on.finish?.(href);
  }

  /** Run a script the core sent, in the page. */
  run(script) {
    this.win.eval(script);
  }

  destroy() {
    clearInterval(this.poll);
    const frames = this.rt.controller.frames;
    const i = frames.indexOf(this.frame);
    if (i >= 0) frames.splice(i, 1);
  }
}

// -------------------------------------------------------------------- the screen

function makeScreen() {
  const cancel = h('button', { class: 'ghostbtn' }, 'Cancelar');
  const wait = h('div', { class: 'sjwait' }, h('p', { class: 'body t2' }, 'Abrindo a página da Microsoft…'), skeleton(220, 24), skeleton(320, 15), skeleton(280, 15));
  const iframe = h('iframe', { class: 'sjframe', title: 'Página de login da Microsoft', referrerpolicy: 'no-referrer' });
  const stage = h('div', { class: 'loginstage sjstage' }, iframe, wait);
  const root = h(
    'div',
    { class: 'loginscreen sj stash', role: 'dialog', 'aria-label': 'Entrar na conta' },
    h('div', { class: 'loginbar' }, icon('xbox', 'loginic'), h('span', { class: 'row-title' }, 'Entrar na conta'), h('span', { class: 'grow' }), cancel),
    stage,
  );
  document.body.append(root);
  return { root, stage, iframe, wait, cancel };
}

// ------------------------------------------------------------------ the entry

let slot = null; // the sign-in begun ahead, out of sight: an Entry
let active = false; // a sign-in of this kind is on the screen

/** One sign-in's screen, and the page that will fill it (`ready` resolves when the page is loading in it). */
class Entry {
  constructor() {
    this.screen = makeScreen();
    this.rt = null;
    this.page = null;
    this.ready = null;
  }
}

/** Begin a sign-in: its screen at once (out of sight), the runtime and the page on the way. */
function begin() {
  mark('build:start');
  const entry = new Entry();
  dbg.entry = entry;
  entry.ready = (async () => {
    // The page's address and headers do not wait for the runtime: both are asked for at once.
    const specP = withTimeout(api.loginDirectPage(), 10_000, 'a página do login');
    specP.catch(() => {});
    const rt = await withTimeout(loadRuntime(), RUNTIME_TIMEOUT, 'o login no navegador');
    rt.Plugin ||= pluginClass(rt.ManagedPlugin);
    await wipeState(rt.controller);
    const spec = await specP;
    entry.rt = rt;
    entry.page = new Page(rt, spec, entry.screen);
    entry.page.load();
    mark('build:done');
    return entry;
  })();
  entry.ready.catch(() => {
    if (slot === entry) slot = null;
    teardown(entry);
  });
  return entry;
}

/** Load the page ahead, out of sight. Idempotent; a no-op where this way of signing in is not wanted. */
export function preload() {
  if (slot || active || !wanted() || document.visibilityState === 'hidden') return;
  const mine = (slot = begin());
  setTimeout(() => {
    if (slot === mine) {
      slot = null;
      teardown(mine);
    }
  }, KEEP_PRELOADED);
}

function teardown(e) {
  e.page?.destroy();
  e.screen.root.remove();
  if (!active) document.documentElement.classList.remove('logging');
  if (e.rt) wipeState(e.rt.controller);
}

/**
 * Sign in this way. Resolves when the sign-in is over (done, or cancelled), or with 'retry' when the
 * person asked to try again; rejects with `Fallback` when the other way should be used.
 */
export async function startDirect() {
  if (active) throw new Fallback('já há um login na tela');
  preload();
  const entry = slot;
  slot = null;
  if (!entry) throw new Fallback('login no navegador indisponível');
  active = true;
  const { screen } = entry;
  // Shown at once: if the page was loaded ahead it is already there; if not, the screen says it is on the way.
  screen.root.classList.remove('stash');
  document.documentElement.classList.add('logging');
  mark('start:shown');

  return new Promise((resolve, reject) => {
    let id = null;
    let over = false;
    let chain = Promise.resolve();
    let watchdog = 0;

    function end(outcome) {
      if (over) return;
      over = true;
      clearTimeout(watchdog);
      active = false;
      if (id) api.loginDirectCancel(id).catch(() => {});
      teardown(entry);
      if (outcome instanceof Fallback) reject(outcome);
      else resolve(outcome);
    }

    function failed(title, detail, actionLabel, onAction) {
      screen.wait.classList.add('off');
      screen.stage.replaceChildren(stateBlock({ title, detail, actionLabel, onAction, tone: 'danger' }));
    }
    const retry = () => end('retry');

    screen.cancel.onclick = () => end();
    // The page has not opened in time: the other way.
    watchdog = setTimeout(() => {
      if (!entry.page?.ready && !over) end(new Fallback('a página da Microsoft não abriu a tempo'));
    }, FIELD_TIMEOUT);

    entry.ready.then(
      () => wire(),
      (e) => end(e instanceof Fallback ? e : new Fallback(e?.message || String(e))),
    );

    function wire() {
      if (over) return;
      const { page } = entry;
      mark('start:wired');

      // Register the device with Microsoft now, while the person types; messages that need the core wait for it.
      const idP = api.loginDirectStart().then(
        (v) => {
          id = v;
          mark('start:device');
          if (over) api.loginDirectCancel(v).catch(() => {});
          return v;
        },
        (e) => {
          failed('Não foi possível abrir o login', e.message || String(e), 'Voltar', () => end());
          throw e;
        },
      );
      idP.catch(() => {});

      async function finished(r) {
        if (over) return;
        over = true;
        clearTimeout(watchdog);
        active = false;
        id = null; // done: nothing left to cancel
        teardown(entry);
        toast(r.warning || 'Conta conectada', r.warning ? 'err' : '');
        try {
          await (await import('./actions.js')).refresh();
        } finally {
          resolve();
        }
      }

      // What the core says to do with a message of the page: run a script in it, load another page, or it is done.
      const handle = async (body) => {
        const sid = await idP;
        if (over) return;
        const r = await api.loginDirectMessage(sid, body);
        if (over) return;
        mark(`core:${r.action}`);
        if (r.action === 'eval') page.run(r.script);
        else if (r.action === 'open') page.frame.go(r.url);
        else if (r.action === 'done') await finished(r);
      };
      page.on.message = (body) => {
        chain = chain
          .then(() => handle(body))
          .catch((e) => {
            if (!over) failed('Não foi possível entrar', e.message || String(e), 'Tentar de novo', retry);
          });
      };
      // On the sign-in's last page the core's script hands the page's data to the bridge (and so to the line above).
      page.on.finish = () => {
        screen.wait.classList.remove('off');
        screen.wait.replaceChildren(h('div', { class: 'section' }, 'Conectando à Xbox Live'), h('p', { class: 'body t2' }, 'Validando o login do Windows e lendo o perfil.'));
        try {
          page.run(page.spec.finish_script);
        } catch (e) {
          failed('Não foi possível entrar', e.message || String(e), 'Tentar de novo', retry);
        }
      };
      page.on.ready = () => screen.wait.classList.add('off');

      // A page loaded ahead and left for long enough is loaded again (its flow would be stale).
      if (Date.now() - page.loadedAt > PAGE_LIFETIME) page.load();
      else if (page.ready) screen.wait.classList.add('off');
    }
  });
}
