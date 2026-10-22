// The password page: send the password, and the server answers with the session cookie.
import { ICONS } from './icons.js';

// The app's own Xbox mark (the same constant the app draws), not a copy of it.
document.getElementById('ic').innerHTML = ICONS.xbox;

const form = document.getElementById('f');
const pw = document.getElementById('pw');
const msg = document.getElementById('msg');
const btn = form.querySelector('button');

function say(text) {
  msg.textContent = text;
  msg.hidden = !text;
}

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  say('');
  btn.disabled = true;
  try {
    const r = await fetch('/api/unlock', {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-xau': '1' },
      body: JSON.stringify({ password: pw.value }),
      credentials: 'same-origin',
    });
    const j = await r.json().catch(() => ({}));
    if (r.ok && j.ok) {
      location.replace('/');
      return;
    }
    if (r.status === 429) {
      const min = Math.max(1, Math.ceil((j.retry || 60) / 60));
      say(`Tentativas demais. Tente de novo em ${min} min.`);
    } else {
      say(j.err || 'Senha incorreta.');
      pw.select();
    }
  } catch {
    say('Não consegui falar com o servidor.');
  } finally {
    btn.disabled = false;
  }
});
