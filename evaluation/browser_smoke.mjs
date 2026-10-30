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
const delivered = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'))).items.filter(i => i.state === 'succeeded');
const expectedNotes = delivered.map(item => {
  const frozen = JSON.parse(fs.readFileSync(path.join(root, item.jobId, 'manifest.json')));
  return { title: frozen.snapshot.sequences.find(s => s.id === frozen.sequenceId).name,
    objective: frozen.editPreflight?.editPlan?.brief?.objective ?? null,
    reasons: frozen.editPreflight?.editPlan?.decisions?.slice(0, 20).map(d => d.reason) ?? [] };
});
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
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: base });
    await context.route('**/*', route => route.request().url().startsWith(base + '/')
      ? route.continue() : route.abort());
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base + '/');
    const shownNotes = await page.locator('section').evaluateAll(sections => sections.map(section => ({
      title: section.querySelector('h2')?.textContent ?? null,
      objective: section.querySelector('[data-edit-objective]')?.textContent ?? null,
      reasons: [...section.querySelectorAll('[data-cut-reason]')].map(li => li.textContent),
    })));
    if ((expectedNotes.some(note => note.objective !== null) || shownNotes.some(note => note.title !== null))
      && JSON.stringify(shownNotes) !== JSON.stringify(expectedNotes)) {
      throw new Error('Frozen edit notes were changed or interpreted as markup');
    }
    if (await page.evaluate(() => window.unexpectedMarkup === true)) throw new Error('Author text executed as script');
    await page.waitForFunction(() => [...document.querySelectorAll('video')]
      .every(video => video.readyState >= 2));
    const media = await page.evaluate(async () => {
      const videos = [];
      for (const video of document.querySelectorAll('video')) {
        const initial = video.currentTime;
        await video.play();
        const started = performance.now();
        await new Promise((resolve, reject) => {
          const progress = () => {
            if (video.currentTime > initial + 0.05) {
              clearTimeout(timeout);
              video.removeEventListener('timeupdate', progress);
              resolve();
            }
          };
          const timeout = setTimeout(() => {
            video.removeEventListener('timeupdate', progress);
            reject(new Error('video clock did not advance within 10 seconds'));
          }, 10000);
          video.addEventListener('timeupdate', progress);
          progress();
        });
        const playbackWaitMs = performance.now() - started;
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
          played, playbackWaitMs, seekTargetSeconds: target, seekSeconds: video.currentTime,
          error: video.error?.code ?? null, displayedWidth: video.getBoundingClientRect().width });
      }
      return { videos, viewport: window.innerWidth, pageWidth: document.documentElement.scrollWidth };
    });
    const feedback = [];
    for (const [index, item] of delivered.entries()) {
      const section = page.locator('section').nth(index);
      const reference = section.locator('textarea');
      if (!(await section.locator('[data-feedback-reference]').isVisible())
        || !(await reference.getAttribute('readonly') === '')
        || await section.locator('video').getAttribute('aria-labelledby') !== `title-${item.jobId}`) {
        throw new Error('Feedback reference or accessible video name is missing');
      }
      const ms = Math.round(media.videos[index].seekSeconds * 1000);
      const timestamp = [Math.floor(ms / 3600000), Math.floor(ms / 60000) % 60, Math.floor(ms / 1000) % 60]
        .map(part => String(part).padStart(2, '0')).join(':') + '.' + String(ms % 1000).padStart(3, '0');
      if (await section.locator('[data-feedback-time]').textContent() !== timestamp) {
        throw new Error('Displayed playback time differs from the video position');
      }
      const title = expectedNotes[index].title.replace(/\s+/g, ' ').trim() || 'Video';
      const expected = `Video: ${title}\nRender job ID: ${item.jobId}\nRendered revision: ${item.revision}\nPlayback time: ${timestamp} (${ms} ms)`;
      if (await reference.inputValue() !== expected) throw new Error('Feedback reference lost the frozen identity or playback time');
      const fallback = label === 'small-phone' && index === 0;
      if (fallback) await page.evaluate(() => Object.defineProperty(navigator.clipboard, 'writeText', {
        configurable: true, value: async () => { throw new Error('clipboard denied'); },
      }));
      await section.getByRole('button', { name: 'Copy feedback reference' }).click();
      await section.locator('[role=status]').filter({ hasText: fallback ? 'Reference selected' : 'Feedback reference copied' }).waitFor();
      if (fallback) {
        const selected = await reference.evaluate(element => document.activeElement === element
          && element.selectionStart === 0 && element.selectionEnd === element.value.length);
        if (!selected || !(await section.locator('[role=status]').textContent()).includes('Reference selected')) {
          throw new Error('Clipboard denial did not select the reference for manual copying');
        }
        await page.evaluate(() => { delete navigator.clipboard.writeText; });
      } else {
        const copied = await page.evaluate(() => navigator.clipboard.readText());
        if (copied !== expected) {
          throw new Error('Copied feedback reference differs from the shown timestamp: ' + JSON.stringify({ copied, expected }));
        }
      }
      feedback.push({ jobId: item.jobId, revision: item.revision, milliseconds: ms, fallback });
    }
    results.push({ label, requestedViewport: viewport, ...media, editingNotes: shownNotes, feedback, pageErrors: errors });
    await page.screenshot({ path: path.join(output, label + '.png'), fullPage: true });
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
    await context.close();
  }
  const noScript = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  const fallbackPage = await noScript.newPage();
  await fallbackPage.goto(base + '/');
  for (const [index, item] of delivered.entries()) {
    const section = fallbackPage.locator('section').nth(index);
    const notice = section.locator('noscript');
    if (!(await notice.isVisible()) || !(await notice.textContent()).includes(item.jobId)
      || !(await notice.textContent()).includes(`rendered revision ${item.revision}`)
      || await section.locator('[data-feedback-reference]').isVisible()) {
      throw new Error('No-script feedback guidance is missing or interactive controls were exposed');
    }
  }
  await noScript.close();
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify({ passed: true,
    browser: await browser.version(), results, requests, noScriptFeedbackGuidance: true,
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
