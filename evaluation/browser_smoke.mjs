// Playback, seeking and responsive layout of an actual generated review bundle.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [executable, review, output, playwrightRoot] = process.argv.slice(2);
if (!playwrightRoot) throw new Error('Usage: node browser_smoke.mjs CHROMIUM REVIEW_DIRECTORY NEW_OUTPUT PLAYWRIGHT_ROOT');
const { chromium } = await import(pathToFileURL(path.join(playwrightRoot, 'index.mjs')));
const root = fs.realpathSync(review);
fs.mkdirSync(output, { recursive: false });
const requests = [];
const server = http.createServer((request, response) => {
  try {
    const relative = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const file = fs.realpathSync(path.resolve(root, '.' + (relative === '/' ? '/index.html' : relative)));
    if (!file.startsWith(root + path.sep) || !fs.statSync(file).isFile()) {
      response.writeHead(403); response.end(); return;
    }
    const size = fs.statSync(file).size;
    const types = { '.html': 'text/html', '.mp4': 'video/mp4', '.vtt': 'text/vtt',
      '.json': 'application/json', '.png': 'image/png', '.srt': 'text/plain' };
    const headers = { 'Content-Type': types[path.extname(file)] ?? 'application/octet-stream',
      'Accept-Ranges': 'bytes' };
    let start = 0, end = size - 1, status = 200;
    if (request.headers.range) {
      const match = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range);
      if (!match) { response.writeHead(416); response.end(); return; }
      start = Number(match[1]); end = match[2] ? Number(match[2]) : end;
      if (start >= size || end >= size || start > end) {
        response.writeHead(416, { 'Content-Range': `bytes */${size}` }); response.end(); return;
      }
      headers['Content-Range'] = `bytes ${start}-${end}/${size}`;
      status = 206;
    }
    headers['Content-Length'] = end - start + 1;
    requests.push({ path: relative, status, range: request.headers.range ?? null });
    response.writeHead(status, headers);
    if (request.method === 'HEAD' || size === 0) response.end();
    else fs.createReadStream(file, { start, end }).on('error', () => response.destroy()).pipe(response);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = 'http://127.0.0.1:' + server.address().port;
let browser;
const results = [];
try {
  browser = await chromium.launch({ executablePath: executable, headless: true, args: ['--no-sandbox'] });
  for (const [label, viewport, mobile] of [
    ['desktop', { width: 1280, height: 900 }, false],
    ['phone', { width: 390, height: 844 }, true],
    ['small-phone', { width: 320, height: 640 }, true],
  ]) {
    const context = await browser.newContext({ viewport, isMobile: mobile, hasTouch: mobile });
    await context.route('**/*', route => route.request().url().startsWith(base + '/')
      ? route.continue() : route.abort());
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base + '/');
    await page.waitForFunction(() => [...document.querySelectorAll('video')]
      .every(video => video.readyState >= 2));
    const media = await page.evaluate(async () => {
      const videos = [];
      for (const video of document.querySelectorAll('video')) {
        const initial = video.currentTime;
        await video.play();
        await new Promise(resolve => setTimeout(resolve, 350));
        const played = video.currentTime > initial;
        video.pause();
        const target = Math.min(2, video.duration / 2);
        const sought = new Promise((resolve, reject) => {
          const timeout = setTimeout(() => reject(new Error('seek did not finish')), 5000);
          video.addEventListener('seeked', () => { clearTimeout(timeout); resolve(); }, { once: true });
        });
        video.currentTime = target;
        await sought;
        videos.push({ width: video.videoWidth, height: video.videoHeight, duration: video.duration,
          played, seekTargetSeconds: target, seekSeconds: video.currentTime,
          error: video.error?.code ?? null, displayedWidth: video.getBoundingClientRect().width });
      }
      return { videos, viewport: window.innerWidth, pageWidth: document.documentElement.scrollWidth };
    });
    results.push({ label, requestedViewport: viewport, ...media, pageErrors: errors });
    if (errors.length || !media.videos.length || media.videos.some(video => !video.played
      || video.error || Math.abs(video.seekSeconds - video.seekTargetSeconds) > 0.05)) {
      throw new Error('Playback/seek failed: ' + JSON.stringify(results.at(-1)));
    }
    if (media.viewport !== viewport.width || media.pageWidth > viewport.width + 1
      || media.videos.some(video => video.displayedWidth > viewport.width)) {
      throw new Error('Responsive layout failed: ' + JSON.stringify(results.at(-1)));
    }
    for (const href of await page.locator('a').evaluateAll(links => links.map(link => link.href))) {
      if (!href.startsWith(base + '/')) throw new Error('Review links leave bundle');
      const response = await context.request.get(href);
      if (!response.ok()) throw new Error('Review link unavailable: ' + href);
      if (href.endsWith('.json')) JSON.parse(await response.text());
      if (href.endsWith('.vtt') && !(await response.text()).startsWith('WEBVTT')) {
        throw new Error('Invalid linked captions');
      }
    }
    await page.screenshot({ path: path.join(output, label + '.png'), fullPage: true });
    await context.close();
  }
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify({ passed: true,
    browser: await browser.version(), results, requests,
    limitations: ['Chromium viewport emulation; no iOS Safari, physical phone, HDR display or real-camera appearance approval'],
  }, null, 2));
  console.log('Browser playback/layout passed:', path.join(output, 'summary.json'));
} catch (error) {
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify({ passed: false,
    error: String(error), results, requests }, null, 2));
  throw error;
} finally {
  if (browser) await browser.close();
  server.close();
}
