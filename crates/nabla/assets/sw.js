var cacheName = 'nabla';
var filesToCache = [
  './',
  './index.html',
  './nabla.js',
  './nabla_bg.wasm',
  './synth-worklet.js',
  './nabla_synth.wasm',
];

/* Cache the app's content for offline use */
self.addEventListener('install', function (e) {
  e.waitUntil(
    caches.open(cacheName).then(function (cache) {
      return cache.addAll(filesToCache);
    })
  );
});

/* Network first, so new deployments show up at once; the cache is only an offline fallback */
self.addEventListener('fetch', function (e) {
  if (e.request.method !== 'GET') {
    return;
  }
  e.respondWith(
    fetch(e.request)
      .then(function (response) {
        var copy = response.clone();
        caches.open(cacheName).then(function (cache) {
          cache.put(e.request, copy);
        });
        return response;
      })
      .catch(function () {
        return caches.match(e.request);
      })
  );
});
