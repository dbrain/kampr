// Kampr's service worker.
//
// Four jobs, and no others. It shows a push. It opens the right pane when one is tapped. It warms
// a small cache on the way to both, so the tap lands on data rather than on a load. And it serves
// exactly those two warm URLs back to the page.
//
// It deliberately does **not** proxy the app's own fetches. The wasm bundle is served immutable
// and the browser already caches it; a caching fetch handler in front of that would be a second,
// worse cache, and a way to serve a stale bundle after a node upgrade.

const CACHE = 'kampr-warm-v1';
const DB = 'kampr';
const STORE = 'auth';

// Notifications are tagged, so the newest replaces the last rather than stacking a column of
// stale prompts on a phone that was away. Must match `kampr_push::note::TAG`.
const TAG = 'kampr.blocked';

// The only two URLs this worker caches or serves. Anything else goes straight to the network.
const WARM = ['/api/node', '/api/warm'];

self.addEventListener('install', (event) => {
  // A new worker takes over at once. The alternative is a node upgrade whose new push payload is
  // handled by the old worker until every tab happens to close.
  event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    const names = await caches.keys();
    await Promise.all(names.filter((n) => n !== CACHE).map((n) => caches.delete(n)));
    await self.clients.claim();
  })());
});

// --- the device token -------------------------------------------------------------------------
//
// A service worker cannot read localStorage, and it outlives every page, so the token is handed
// over by the page and kept in IndexedDB. A cookie would have done it in fewer lines and would
// have added a CSRF surface to a node that currently has none.

function idb() {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => open.result.createObjectStore(STORE);
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error);
  });
}

async function withStore(mode, run) {
  const db = await idb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, mode);
    const request = run(tx.objectStore(STORE));
    tx.oncomplete = () => resolve(request ? request.result : undefined);
    tx.onerror = () => reject(tx.error);
  });
}

const token = {
  get: () => withStore('readonly', (s) => s.get('token')).catch(() => undefined),
  set: (value) => withStore('readwrite', (s) =>
    value ? s.put(value, 'token') : s.delete('token')).catch(() => undefined),
};

self.addEventListener('message', (event) => {
  const message = event.data || {};
  if (message.type === 'kampr-token') {
    event.waitUntil(token.set(message.token || null));
  }
});

async function authorized(url) {
  const bearer = await token.get();
  return fetch(url, {
    credentials: 'same-origin',
    headers: bearer ? { Authorization: 'Bearer ' + bearer } : {},
  });
}

// --- push -------------------------------------------------------------------------------------

self.addEventListener('push', (event) => {
  event.waitUntil(handlePush(event.data));
});

async function handlePush(data) {
  let note = null;
  try {
    note = data ? data.json() : null;
  } catch (_) {
    note = null;
  }
  if (!note || !note.title) {
    // A push with no readable payload still means something wants you. Saying so beats swallowing
    // it — and a browser may revoke the permission of a worker that shows nothing at all.
    note = { title: 'An agent needs you', body: 'Open Kampr to see which', panes: [] };
  }

  // Warm first, then show. The phone is about to be tapped, and these are a few kilobytes against
  // a notification the user has not finished reading.
  await warm(note);

  await self.registration.showNotification(note.title, {
    body: note.body || '',
    tag: note.tag || TAG,
    renotify: true,
    icon: '/icons/kampr-192.png',
    badge: '/icons/kampr-192.png',
    timestamp: Date.now(),
    data: {
      pane: note.pane || null,
      count: note.count || (note.panes || []).length || 1,
    },
  });
}

// Findings §3.11: a reconnect costs exactly one full frame and a full grid is ~4 KB, so
// prefetching the herd and the blocked pane's own state is cheap enough to do on every push.
// It fails quietly — a warm cache is an optimisation, and a cold open still works.
async function warm(note) {
  const panes = (note.panes || []).map((p) => p.pane).filter(Boolean);
  const targets = ['/api/node'].concat(
    panes.slice(0, 4).map((p) => '/api/warm?pane=' + encodeURIComponent(p)),
  );
  if (panes.length === 0) targets.push('/api/warm');
  try {
    const cache = await caches.open(CACHE);
    await Promise.all(targets.map(async (url) => {
      try {
        const response = await authorized(url);
        if (response.ok) await cache.put(url, response.clone());
      } catch (_) {
        // Offline, or the tunnel is not up yet — which on Android is exactly when a tap arrives
        // before the socket does, and exactly why the notification body carries the question.
      }
    }));
  } catch (_) {
  }
}

// --- serving the warm cache back --------------------------------------------------------------

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== 'GET' || url.origin !== self.location.origin) return;
  if (!WARM.includes(url.pathname)) return;
  event.respondWith(warmFirst(event.request));
});

// Cache first, network behind it. The cached copy is at most a notification old and the page
// replaces it from the live socket within a second either way, so serving it instantly is the
// whole point — a spinner in front of data we already hold is the thing being removed.
async function warmFirst(request) {
  const cache = await caches.open(CACHE);
  const key = new URL(request.url).pathname + new URL(request.url).search;
  const hit = await cache.match(key);
  const live = fetch(request).then(async (response) => {
    if (response.ok) await cache.put(key, response.clone());
    return response;
  });
  if (hit) {
    live.catch(() => {});
    return hit;
  }
  return live;
}

// --- the deep link ----------------------------------------------------------------------------

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  event.waitUntil(open(event.notification.data || {}));
});

// One blocked pane opens that pane in its conversation view — the view an answer can be given
// from without leasing a terminal. A batch opens the triage list, because picking one of three
// for the user would be picking wrong two times in three.
async function open(data) {
  const url = data.pane
    ? '/?pane=' + encodeURIComponent(data.pane) + '&view=conversation'
    : '/?screen=herd';
  const clients = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
  for (const client of clients) {
    if ('focus' in client) {
      // A live tab is navigated rather than replaced: it already holds a warm grid and an open
      // socket, and reloading would throw both away to show the same thing.
      if ('navigate' in client) {
        try {
          await client.navigate(url);
        } catch (_) {
        }
      }
      return client.focus();
    }
  }
  return self.clients.openWindow(url);
}

// A browser may rotate a subscription with no page open. Without this the device simply goes
// quiet and nothing says why.
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil((async () => {
    const subscription = event.newSubscription
      || (event.oldSubscription
        ? await self.registration.pushManager.subscribe(event.oldSubscription.options)
        : null);
    if (!subscription) return;
    const bearer = await token.get();
    try {
      await fetch('/api/push/subscribe', {
        method: 'POST',
        credentials: 'same-origin',
        headers: Object.assign(
          { 'Content-Type': 'application/json' },
          bearer ? { Authorization: 'Bearer ' + bearer } : {},
        ),
        body: JSON.stringify(subscription.toJSON()),
      });
    } catch (_) {
    }
  })());
});
