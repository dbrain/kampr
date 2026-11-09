import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('./src/wasmJsMain/resources/sw.js', import.meta.url), 'utf8');

function worker() {
  const listeners = {};
  const self = {
    location: { origin: 'https://node.test' },
    addEventListener: (type, fn) => { listeners[type] = fn; },
  };
  vm.runInNewContext(source, { self, URL, Request: class {}, caches: { open: async () => ({ match: async () => undefined, put: async () => {} }) }, fetch: () => Promise.reject(new Error('offline')) });
  return (url, headers = {}) => {
    let answered = false;
    listeners.fetch({
      request: { url, method: 'GET', mode: 'cors', headers: new Headers(headers) },
      respondWith: (response) => {
        answered = true;
        response.catch(() => {});
      },
    });
    return answered;
  };
}

test('the warm herd is served from the cache', () => {
  assert.equal(worker()('https://node.test/api/node'), true);
});

test('a clock reading always goes to the network, because a cached Date header is that old', () => {
  assert.equal(worker()('https://node.test/api/node', { 'x-kampr-clock': '1' }), false);
});
