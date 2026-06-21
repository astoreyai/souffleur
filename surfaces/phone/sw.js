// Minimal service worker: caches the app shell so the surface opens offline.
// The live data is a WebSocket (never cached).
const SHELL = ["./", "./index.html", "./manifest.json"];
self.addEventListener("install", (e) => {
  e.waitUntil(caches.open("souffleur-v1").then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
self.addEventListener("fetch", (e) => {
  if (e.request.method !== "GET") return;
  e.respondWith(caches.match(e.request).then((hit) => hit || fetch(e.request)));
});
