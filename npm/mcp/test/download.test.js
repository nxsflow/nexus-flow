'use strict';
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { resolveRedirect, download } = require('../lib/download');

let server;
let base;

before(async () => {
  server = http.createServer((req, res) => {
    if (req.url === '/latest') {
      res.writeHead(302, { Location: `${base}/download/ok.bin` });
      return res.end();
    }
    if (req.url === '/download/ok.bin') {
      res.writeHead(200, { 'Content-Type': 'application/octet-stream' });
      return res.end(Buffer.from('the payload bytes'));
    }
    if (req.url === '/redir1') {
      res.writeHead(302, { Location: `${base}/redir2` });
      return res.end();
    }
    if (req.url === '/redir2') {
      res.writeHead(302, { Location: `${base}/download/ok.bin` });
      return res.end();
    }
    if (req.url === '/missing') {
      res.writeHead(404);
      return res.end('nope');
    }
    if (req.url === '/boom') {
      res.writeHead(500);
      return res.end('server error');
    }
    if (req.url === '/stall') {
      // Accept, send a byte, then never finish — the slow-loris / hung-TLS shape.
      res.writeHead(200);
      res.write('x');
      return; // no res.end(), ever
    }
    if (req.url === '/big') {
      res.writeHead(200);
      return res.end(Buffer.alloc(100_000));
    }
    if (req.url === '/redir-offsite') {
      res.writeHead(302, { Location: 'http://other.invalid/x' });
      return res.end();
    }
    res.writeHead(404);
    res.end();
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

test('resolveRedirect returns the Location of a 302 without following it', async () => {
  assert.equal(await resolveRedirect(`${base}/latest`), `${base}/download/ok.bin`);
});

test('resolveRedirect returns null on a 404 (nothing published)', async () => {
  assert.equal(await resolveRedirect(`${base}/missing`), null);
});

test('resolveRedirect throws on a non-3xx/404 status (no silent downgrade)', async () => {
  await assert.rejects(() => resolveRedirect(`${base}/boom`), /500|unexpected/i);
});

test('download fetches a 200 body', async () => {
  assert.deepEqual(await download(`${base}/download/ok.bin`), Buffer.from('the payload bytes'));
});

test('download follows a redirect chain', async () => {
  assert.deepEqual(await download(`${base}/redir1`), Buffer.from('the payload bytes'));
});

test('download throws on a 404', async () => {
  await assert.rejects(() => download(`${base}/missing`), /404|failed/i);
});

test('download times out on a stalled response instead of hanging forever', async () => {
  await assert.rejects(
    () => download(`${base}/stall`, { timeoutMs: 150 }),
    /timed out|timeout/i
  );
});

test('download aborts when the response exceeds the byte cap (pre-verify DoS guard)', async () => {
  await assert.rejects(
    () => download(`${base}/big`, { maxBytes: 1000 }),
    /too large|exceeds|byte/i
  );
});

test('download refuses a cross-origin redirect (unguarded server-directed fetch)', async () => {
  await assert.rejects(
    () => download(`${base}/redir-offsite`),
    /origin|refus/i
  );
});

test('resolveRedirect times out on a stalled response', async () => {
  await assert.rejects(
    () => resolveRedirect(`${base}/stall`, { timeoutMs: 150 }),
    /timed out|timeout/i
  );
});
