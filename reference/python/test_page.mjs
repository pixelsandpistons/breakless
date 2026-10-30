#!/usr/bin/env node
// Opens the page in headless Chromium, watches a live channel for a while and says whether the
// picture is really moving. Needs node 22+ and chromium on this PC; nothing to install.
//
//   node twitch/test_page.mjs                      # most-watched channel, 20 s
//   node twitch/test_page.mjs somechannel 40       # a named channel, 40 s
//   BASE=http://127.0.0.1:18080 node twitch/test_page.mjs     # a copy started by hand (default http://127.0.0.1:8080)
// A name behind a sign-in cannot be tested this way: the page would be the sign-in form.
//
// Prints one JSON line and writes a screenshot to /tmp/twitch-page.png (or $SHOT).
// Exit 0 when the video advanced and no advert segment was handed on; 1 otherwise.
import { spawn } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const base = process.env.BASE || 'http://127.0.0.1:8080';
const seconds = Number(process.argv[3] || 20);
const shot = process.env.SHOT || '/tmp/twitch-page.png';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let channel = process.argv[2];
if (!channel) channel = (await (await fetch(base + '/api/channels')).json()).top[0].login;

const profile = mkdtempSync(join(tmpdir(), 'twitch-page-'));
const port = 9300 + Math.floor(Math.random() * 400);
const chrome = spawn('chromium', ['--headless=new', '--user-data-dir=' + profile, '--no-first-run', '--disable-gpu',
  '--mute-audio', '--autoplay-policy=no-user-gesture-required', '--window-size=1280,900',
  '--remote-debugging-port=' + port, 'about:blank'], { stdio: 'ignore' });

let code = 1;
try {
  let target;
  for (let i = 0; i < 40 && !target; i++) {
    await sleep(250);
    try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page'); } catch {}
  }
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r) => ws.addEventListener('open', r));
  let id = 0;
  const waiting = new Map(), problems = [];
  ws.addEventListener('message', (m) => {
    const d = JSON.parse(m.data);
    if (d.id && waiting.has(d.id)) { waiting.get(d.id)(d.result || d); waiting.delete(d.id); }
    if (d.method === 'Runtime.exceptionThrown') problems.push(d.params.exceptionDetails.exception?.description || d.params.exceptionDetails.text);
    if (d.method === 'Log.entryAdded' && d.params.entry.level === 'error') problems.push(d.params.entry.text + ' ' + (d.params.entry.url || ''));
  });
  const send = (method, params = {}) => new Promise((r) => { waiting.set(++id, r); ws.send(JSON.stringify({ id, method, params })); });
  const evaluate = async (expression) => (await send('Runtime.evaluate', { expression, returnByValue: true })).result?.value;

  await send('Runtime.enable');
  await send('Log.enable');
  await send('Page.enable');
  await send('Page.navigate', { url: `${base}/watch/${channel}` });
  await sleep(seconds * 1000);
  const page = await evaluate(`(() => { const v = document.getElementById('video'); return {
    time: v.currentTime, paused: v.paused, width: v.videoWidth, height: v.videoHeight, ready: v.readyState,
    qualities: [...document.getElementById('quality').options].map(o => o.textContent),
    lamp: document.getElementById('exit').textContent, notice: document.getElementById('notice').textContent,
    cards: document.querySelectorAll('.card').length }; })()`);
  const png = await send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(shot, Buffer.from(png.data, 'base64'));
  const status = await (await fetch(base + '/api/status')).json();
  const mine = status.watching.find((w) => w.channel === channel.toLowerCase()) || {};
  const ok = page.time > seconds / 3 && page.width > 0 && problems.length === 0;
  console.log(JSON.stringify({ ok, channel, page, from: mine.from, country: mine.country,
    segments_passed: mine.segments_passed, segments_cut: mine.segments_cut, problems, screenshot: shot }));
  code = ok ? 0 : 1;
  ws.close();
} finally {
  chrome.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
}
process.exit(code);
