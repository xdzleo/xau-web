// The farm's notices (finished, level-up) as browser notifications, with an
// in-page toast as the fallback. Each is a switch in Ajustes.

import { prefs } from './state.js';
import { toast } from './ui.js';

export const supported = 'Notification' in window;
export const permission = () => (supported ? Notification.permission : 'denied');

export const enabled = {
  done: () => prefs.get('notifyDone', true),
  level: () => prefs.get('notifyLevel', true),
};

export async function ask() {
  if (!supported) return 'denied';
  try {
    return await Notification.requestPermission();
  } catch {
    return Notification.permission;
  }
}

/** `kind` is "done" or "level"; a switched-off kind says nothing at all. */
export function notify(kind, title, body, tag) {
  if (!enabled[kind]()) return;
  if (permission() === 'granted' && document.visibilityState !== 'visible') {
    try {
      new Notification(title, { body, tag, icon: '/assets/favicon.svg' });
      return;
    } catch {
      /* fall through to the toast */
    }
  }
  toast(`${title} — ${body}`);
}
