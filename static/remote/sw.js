/* OmniGet Remote service worker: keeps the app shell for offline use.
   API calls and the event socket are never cached here; the app keeps the
   last snapshot itself (localStorage). */
const CACHE = "omniget-remote-v1";
const SHELL = ["/remote/", "/remote/index.html", "/remote/app.js", "/remote/app.css", "/remote/icon.svg", "/remote/manifest.webmanifest"];

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))).then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== "GET" || url.origin !== self.location.origin) return;
  if (!url.pathname.startsWith("/remote/") || url.pathname.startsWith("/remote/api/") || url.pathname.startsWith("/remote/ws")) return;
  // Network first so a new build shows up; the cache answers offline.
  e.respondWith(
    fetch(e.request)
      .then((res) => {
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(e.request, copy));
        }
        return res;
      })
      .catch(() => caches.match(e.request).then((hit) => hit || caches.match("/remote/index.html"))),
  );
});
