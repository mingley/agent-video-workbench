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
    const draftIds = new Set();
    for (const [index, item] of delivered.entries()) {
      const section = page.locator('section').nth(index);
      const pointPanel = section.locator('[data-feedback-reference]');
      const rangePanel = section.locator('[data-feedback-range]');
      const reference = pointPanel.locator('textarea');
      if (!(await pointPanel.isVisible()) || !(await rangePanel.isVisible())
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
      await pointPanel.getByRole('button', { name: 'Copy feedback reference' }).click();
      await pointPanel.locator('[role=status]').filter({ hasText: fallback ? 'Reference selected' : 'Feedback reference copied' }).waitFor();
      if (fallback) {
        const selected = await reference.evaluate(element => document.activeElement === element
          && element.selectionStart === 0 && element.selectionEnd === element.value.length);
        if (!selected || !(await pointPanel.locator('[role=status]').textContent()).includes('Reference selected')) {
          throw new Error('Clipboard denial did not select the reference for manual copying');
        }
        await page.evaluate(() => { delete navigator.clipboard.writeText; });
      } else {
        const copied = await page.evaluate(() => navigator.clipboard.readText());
        if (copied !== expected) {
          throw new Error('Copied feedback reference differs from the shown timestamp: ' + JSON.stringify({ copied, expected }));
        }
      }
      const limitMs = Number(await section.getAttribute('data-review-limit-ms'));
      const availableMs = Math.min(limitMs, Math.floor(media.videos[index].duration * 1000));
      if (!Number.isSafeInteger(limitMs) || availableMs < 4) throw new Error('Fixture is too short for a range check');
      await rangePanel.locator('summary').press('Enter');
      if (!(await rangePanel.locator('textarea').isVisible())) throw new Error('Keyboard could not open range controls');
      const rangeReference = rangePanel.locator('textarea');
      const rangeCopy = rangePanel.getByRole('button', { name: 'Copy range reference' });
      const rangeStatus = rangePanel.locator('[role=status]');
      if (!(await rangeReference.getAttribute('readonly') === '') || !(await rangeCopy.isDisabled())
        || await rangePanel.locator('[data-range-start]').textContent() !== 'Not set'
        || await rangePanel.locator('[data-range-end]').textContent() !== 'Not set') {
        throw new Error('Range controls are not initially empty');
      }
      await rangePanel.getByRole('button', { name: 'Set range end' }).click();
      if (!(await rangeStatus.textContent()).includes('Set a range start first') || !(await rangeCopy.isDisabled())) {
        throw new Error('Range end was accepted without a start');
      }
      const seek = async targetMs => section.locator('video').evaluate(async (video, target) => {
        const seconds = target / 1000;
        if (Math.abs(video.currentTime - seconds) > 0.0001) {
          await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error('range seek did not finish')), 5000);
            video.addEventListener('seeked', () => { clearTimeout(timeout); resolve(); }, { once: true });
            video.currentTime = seconds;
          });
        }
        return Math.round(video.currentTime * 1000);
      }, targetMs);
      if (availableMs >= limitMs) {
        await seek(limitMs);
        await rangePanel.getByRole('button', { name: 'Set range start' }).click();
        if (!(await rangeStatus.textContent()).includes('before the end') || !(await rangeCopy.isDisabled())) {
          throw new Error('A range start at the output end was accepted');
        }
      }
      const startMs = await seek(Math.floor(availableMs / 4));
      await rangePanel.getByRole('button', { name: 'Set range start' }).click();
      await rangePanel.getByRole('button', { name: 'Set range end' }).click();
      if (!(await rangeStatus.textContent()).includes('after the start') || !(await rangeCopy.isDisabled())) {
        throw new Error('A zero-length range was accepted');
      }
      const endMs = await seek(Math.floor(availableMs * 3 / 4));
      await rangePanel.getByRole('button', { name: 'Set range end' }).click();
      const time = value => [Math.floor(value / 3600000), Math.floor(value / 60000) % 60, Math.floor(value / 1000) % 60]
        .map(part => String(part).padStart(2, '0')).join(':') + '.' + String(value % 1000).padStart(3, '0');
      const rangeExpected = `Video: ${title}\nRender job ID: ${item.jobId}\nRendered revision: ${item.revision}\nRange start: ${time(startMs)} (atMs: ${startMs})\nRange end: ${time(endMs)} (endMs: ${endMs})`;
      if (await rangeCopy.isDisabled() || await rangeReference.inputValue() !== rangeExpected
        || await rangePanel.locator('[data-range-start]').textContent() !== time(startMs)
        || await rangePanel.locator('[data-range-end]').textContent() !== time(endMs)) {
        throw new Error('Range lost its frozen identity or exact boundaries');
      }
      if (fallback) await page.evaluate(() => Object.defineProperty(navigator.clipboard, 'writeText', {
        configurable: true, value: async () => { throw new Error('clipboard denied'); },
      }));
      await rangeCopy.click();
      await rangeStatus.filter({ hasText: fallback ? 'Range reference selected' : 'Range reference copied' }).waitFor();
      if (fallback) {
        const selected = await rangeReference.evaluate(element => document.activeElement === element
          && element.selectionStart === 0 && element.selectionEnd === element.value.length);
        if (!selected) throw new Error('Clipboard denial did not select the range reference');
        await page.evaluate(() => { delete navigator.clipboard.writeText; });
      } else if (await page.evaluate(() => navigator.clipboard.readText()) !== rangeExpected) {
        throw new Error('Copied range reference differs from the selected range');
      }
      // Compose a source-bound offline draft from both a range and a point.
      const composer = section.locator('[data-feedback-draft]');
      if (!(await composer.isVisible())) throw new Error('Feedback draft composer is unavailable');
      await composer.locator('summary').press('Enter');
      const draftText = composer.locator('[data-draft-text]');
      const draftActor = composer.locator('[data-draft-actor]');
      const draftJson = composer.locator('[data-draft-json]');
      const draftCopy = composer.getByRole('button', { name: 'Copy feedback draft' });
      const draftDownload = composer.getByRole('button', { name: 'Download feedback draft' });
      const draftStatus = composer.locator('[role=status]');
      const decisionChoice = composer.locator('[data-draft-decision-choice]');
      const setDecision = composer.getByRole('button', { name: 'Set version decision' });
      const clearDecision = composer.getByRole('button', { name: 'Clear decision' });
      if (!(await draftCopy.isDisabled()) || !(await draftDownload.isDisabled())
          || await draftJson.getAttribute('readonly') !== '') throw new Error('Empty draft permits export');
      await setDecision.click();
      if (!(await draftStatus.textContent()).includes('Choose a version decision')
          || !(await draftCopy.isDisabled())) throw new Error('A decision was recorded without choosing it');
      await decisionChoice.selectOption('approved');
      if (!(await draftCopy.isDisabled())) throw new Error('Selecting approval silently recorded a decision');
      await draftActor.fill('🙂'.repeat(51));
      await setDecision.click();
      if (!(await draftStatus.textContent()).includes('200 UTF-8 bytes') || !(await draftCopy.isDisabled())) {
        throw new Error('An oversized reviewer name was accepted for a decision');
      }
      await draftActor.fill('Creator');
      await setDecision.click();
      const decisionOnly = JSON.parse(await draftJson.inputValue());
      if (decisionOnly.jobId !== item.jobId || decisionOnly.artifactRevision !== item.revision
          || decisionOnly.items.length !== 0 || decisionOnly.decision.status !== 'approved'
          || decisionOnly.decision.actor !== 'Creator' || await draftCopy.isDisabled()
          || !(await composer.locator('[data-draft-decision]').textContent()).includes('Approved by Creator')) {
        throw new Error('Decision-only approval lost its explicit frozen identity');
      }
      await draftActor.fill('Changed after approval');
      if (JSON.parse(await draftJson.inputValue()).decision.actor !== 'Creator') {
        throw new Error('Editing the reviewer silently altered the recorded decision');
      }
      await draftCopy.click();
      await draftStatus.filter({ hasText: 'Feedback draft copied' }).waitFor();
      if (JSON.stringify(JSON.parse(await page.evaluate(() => navigator.clipboard.readText())))
          !== JSON.stringify(decisionOnly)) throw new Error('Decision-only approval was not copied');
      if (label === 'desktop' && index === 0) {
        const promisedDecision = page.waitForEvent('download');
        await draftDownload.click();
        const downloadedDecision = await promisedDecision;
        const decisionFile = JSON.parse(fs.readFileSync(await downloadedDecision.path(), 'utf8'));
        if (JSON.stringify(decisionFile) !== JSON.stringify(decisionOnly)) {
          throw new Error('Decision-only approval was not downloaded');
        }
        fs.writeFileSync(path.join(output, 'decision-only.json'), JSON.stringify(decisionFile, null, 2) + '\n');
      }
      if (label === 'small-phone' && index === 0) {
        await composer.screenshot({ path: path.join(output, 'small-phone-decision.png') });
      }
      // A completed/imported decision needs a fresh identity for a later change.
      const newDraft = composer.getByRole('button', { name: 'Start new draft' });
      page.once('dialog', dialog => dialog.dismiss());
      await newDraft.click();
      if (JSON.parse(await draftJson.inputValue()).draftId !== decisionOnly.draftId) {
        throw new Error('Canceling the new-draft warning discarded an existing draft');
      }
      page.once('dialog', dialog => dialog.accept());
      await newDraft.click();
      if (!(await draftCopy.isDisabled()) || await draftJson.inputValue() !== ''
          || !(await composer.locator('[data-draft-decision]').textContent()).includes('None')) {
        throw new Error('Starting a new draft retained a decision or note');
      }
      await decisionChoice.selectOption('changes-requested');
      await setDecision.click();
      const changedDecision = JSON.parse(await draftJson.inputValue());
      if (changedDecision.draftId === decisionOnly.draftId
          || changedDecision.decision.status !== 'changes-requested'
          || changedDecision.decision.actor !== 'Changed after approval') {
        throw new Error('Request-changes did not use a fresh reviewed decision');
      }
      await clearDecision.click();
      if (!(await draftCopy.isDisabled()) || await draftJson.inputValue() !== '') {
        throw new Error('Clearing a decision left an exportable empty draft');
      }
      await draftText.fill('Unsubmitted feedback');
      page.once('dialog', dialog => dialog.dismiss());
      await newDraft.click();
      if (await draftText.inputValue() !== 'Unsubmitted feedback') {
        throw new Error('Canceling a new draft lost unsubmitted feedback');
      }
      await draftText.fill('🙂'.repeat(1500)); // 3000 UTF-16 units, 6000 UTF-8 bytes.
      await composer.getByRole('button', { name: 'Add feedback for marked range' }).click();
      if (!(await draftStatus.textContent()).includes('4000 UTF-8 bytes') || !(await draftCopy.isDisabled())) {
        throw new Error('Oversized Unicode feedback was accepted');
      }
      const actor = 'Reviewer <img src=x onerror="window.unexpectedMarkup=true">';
      const rangeComment = 'Keep this <script>window.unexpectedMarkup=true</script> & the product.';
      await draftActor.fill(actor);
      await draftText.fill(rangeComment);
      await composer.getByRole('button', { name: 'Add feedback for marked range' }).click();
      let draft = JSON.parse(await draftJson.inputValue());
      const frozenManifest = JSON.parse(fs.readFileSync(path.join(root, item.jobId, 'manifest.json')));
      if (draft.schemaVersion !== 1 || draft.projectId !== frozenManifest.snapshot.projectId
          || draft.jobId !== item.jobId || draft.artifactRevision !== item.revision
          || draft.artifactSha256 !== frozenManifest.verification.sha256
          || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(draft.draftId)
          || draft.decision !== undefined
          || JSON.stringify(draft.items) !== JSON.stringify([{ id: 'note-1', atMs: startMs, endMs,
            actor, text: rangeComment }])) {
        throw new Error('Range draft lost its frozen identity or content: ' + JSON.stringify(draft));
      }
      if (draftIds.has(draft.draftId)) throw new Error('Different versions reused a draft identity');
      draftIds.add(draft.draftId);
      if (await composer.locator('[data-draft-list] script, [data-draft-list] img').count()
          || await page.evaluate(() => window.unexpectedMarkup === true)
          || !(await composer.locator('[data-draft-list]').textContent()).includes(rangeComment)) {
        throw new Error('Untrusted feedback was interpreted as markup');
      }
      const pointMs = await seek(Math.floor(availableMs / 2));
      await draftText.fill('Please improve this exact moment.');
      await composer.getByRole('button', { name: 'Add feedback at playback time' }).click();
      draft = JSON.parse(await draftJson.inputValue());
      if (draft.items.length !== 2 || draft.items[1].id !== 'note-2'
          || draft.items[1].atMs !== pointMs || draft.items[1].endMs !== pointMs + 1) {
        throw new Error('Point feedback did not preserve its one-millisecond interval');
      }
      const stableId = draft.draftId;
      await composer.getByRole('button', { name: 'Remove feedback note-1' }).click();
      await draftText.fill('Another review note.');
      await composer.getByRole('button', { name: 'Add feedback at playback time' }).click();
      draft = JSON.parse(await draftJson.inputValue());
      if (draft.draftId !== stableId || JSON.stringify(draft.items.map(note => note.id)) !== '["note-2","note-3"]') {
        throw new Error('Feedback draft or local IDs changed unexpectedly');
      }
      if (fallback) await page.evaluate(() => Object.defineProperty(navigator.clipboard, 'writeText', {
        configurable: true, value: async () => { throw new Error('clipboard denied'); },
      }));
      await draftCopy.click();
      await draftStatus.filter({ hasText: fallback ? 'Feedback draft selected' : 'Feedback draft copied' }).waitFor();
      if (fallback) {
        const selected = await draftJson.evaluate(element => document.activeElement === element
          && element.selectionStart === 0 && element.selectionEnd === element.value.length);
        if (!selected) throw new Error('Clipboard fallback did not select the draft');
        await page.evaluate(() => { delete navigator.clipboard.writeText; });
      } else if (await page.evaluate(() => navigator.clipboard.readText()) !== await draftJson.inputValue()) {
        throw new Error('Copied feedback draft differs from the displayed JSON');
      }
      if (label === 'desktop' && index === 0) {
        const promisedDownload = page.waitForEvent('download');
        await draftDownload.click();
        const download = await promisedDownload;
        const downloaded = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
        if (JSON.stringify(downloaded) !== JSON.stringify(draft)) throw new Error('Downloaded feedback draft changed');
        fs.writeFileSync(path.join(output, 'feedback-draft.json'), JSON.stringify(downloaded, null, 2) + '\n');
      }
      if (label === 'small-phone' && index === 0) {
        await composer.screenshot({ path: path.join(output, 'small-phone-draft.png') });
      }
      await composer.locator('summary').click();
      if (label === 'small-phone' && index === 0) {
        await rangePanel.screenshot({ path: path.join(output, 'small-phone-range.png') });
      }
      // A preview container can expose a few milliseconds beyond the frozen
      // sequence. Simulate that clock tail and require an explicit refusal.
      await section.locator('video').evaluate((video, limit) => Object.defineProperty(video, 'currentTime', {
        configurable: true, get: () => (limit + 10) / 1000,
      }), limitMs);
      await rangePanel.getByRole('button', { name: 'Set range end' }).click();
      if (!(await rangeCopy.isDisabled()) || !(await rangeStatus.textContent()).includes('within the reviewed output')) {
        throw new Error('A range extending beyond the frozen output was accepted');
      }
      await section.locator('video').evaluate(video => { delete video.currentTime; });
      await seek(startMs);
      await rangePanel.getByRole('button', { name: 'Set range end' }).click();
      if (!(await rangeCopy.isDisabled()) || !(await rangeStatus.textContent()).includes('after the start')) {
        throw new Error('Invalid replacement left a stale copyable range');
      }
      await rangePanel.getByRole('button', { name: 'Clear range' }).click();
      if (!(await rangeCopy.isDisabled()) || await rangePanel.locator('[data-range-start]').textContent() !== 'Not set'
        || await rangePanel.locator('[data-range-end]').textContent() !== 'Not set') {
        throw new Error('Clear range retained its previous boundaries');
      }
      await rangePanel.locator('summary').click();
      feedback.push({ jobId: item.jobId, revision: item.revision, milliseconds: ms, range: { startMs, endMs }, fallback });
    }
    const comparison = page.locator('[data-review-compare]');
    if (delivered.length >= 2) {
      if (await comparison.count() !== 1 || !(await comparison.isVisible())
          || await comparison.locator('[data-compare-video]').count() !== 0) {
        throw new Error('Comparison did not defer its players until opened');
      }
      await comparison.locator('summary').press('Enter');
      const selectA = comparison.locator('[data-compare-select="a"]');
      const selectB = comparison.locator('[data-compare-select="b"]');
      const options = await selectA.locator('option').allTextContents();
      if (JSON.stringify(options) !== JSON.stringify(expectedNotes.map(item => item.title))
          || await selectA.inputValue() !== '0' || await selectB.inputValue() !== '1'
          || await comparison.locator('img, script').count() !== 0
          || await page.evaluate(() => window.unexpectedMarkup === true)) {
        throw new Error('Comparison lost version names, defaults, or literal text');
      }
      const playerA = comparison.locator('[data-compare-video="a"]');
      const playerB = comparison.locator('[data-compare-video="b"]');
      const playAndSeek = async player => player.evaluate(async video => {
        if (video.readyState < 1) await new Promise((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error('comparison metadata timed out')), 5000);
          video.addEventListener('loadedmetadata', () => { clearTimeout(timer); resolve(); }, {once:true});
        });
        const start = video.currentTime;
        await video.play();
        await new Promise((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error('comparison playback timed out')), 10000);
          const onTime = () => {
            if (video.currentTime > start + 0.05) {
              clearTimeout(timer); video.removeEventListener('timeupdate', onTime); resolve();
            }
          };
          video.addEventListener('timeupdate', onTime); onTime();
        });
        video.pause();
        const target = Math.min(1, video.duration / 2);
        if (Math.abs(video.currentTime - target) > 0.0001) {
          await new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error('comparison seek timed out')), 5000);
            video.addEventListener('seeked', () => { clearTimeout(timer); resolve(); }, {once:true});
            video.currentTime = target;
          });
        }
        return { target, actual: video.currentTime, controls: video.controls, label: video.getAttribute('aria-label') };
      });
      const playbackA = await playAndSeek(playerA);
      const playbackB = await playAndSeek(playerB);
      if (![playbackA, playbackB].every(result => result.controls && result.label
          && Math.abs(result.actual - result.target) < 0.05)) {
        throw new Error('Independent comparison playback or seek failed');
      }
      for (const [side, item, player] of [['a', delivered[0], playerA], ['b', delivered[1], playerB]]) {
        const identity = await comparison.locator(`[data-compare-identity="${side}"]`).textContent();
        if (!identity.includes(item.jobId) || !identity.includes(`Revision: ${item.revision}`)
            || !(await player.getAttribute('src')).startsWith(`${item.jobId}/`)) {
          throw new Error('Comparison player lost its frozen render identity');
        }
      }
      await playerA.evaluate(video => video.play());
      await selectA.selectOption('1');
      await selectB.selectOption('0');
      if (!(await comparison.locator('[data-compare-identity="a"]').textContent()).includes(delivered[1].jobId)
          || !(await comparison.locator('[data-compare-identity="b"]').textContent()).includes(delivered[0].jobId)
          || !(await playerA.getAttribute('src')).startsWith(`${delivered[1].jobId}/`)
          || !(await playerB.getAttribute('src')).startsWith(`${delivered[0].jobId}/`)
          || !(await playerA.evaluate(video => video.paused))) {
        throw new Error('Selecting different versions failed to update the matching players');
      }
      await playAndSeek(playerA);
      await playAndSeek(playerB);
      if (delivered.length >= 3) {
        await selectA.selectOption('2');
        if (!(await comparison.locator('[data-compare-identity="a"]').textContent()).includes(delivered[2].jobId)
            || !(await playerA.getAttribute('src')).startsWith(`${delivered[2].jobId}/`)) {
          throw new Error('A later version could not be selected for comparison');
        }
        await playAndSeek(playerA);
      }
      const boxA = await comparison.locator('[data-compare-side="a"]').boundingBox();
      const boxB = await comparison.locator('[data-compare-side="b"]').boundingBox();
      if (!boxA || !boxB || (viewport.width > 640 && boxB.x <= boxA.x)
          || (viewport.width <= 640 && boxB.y <= boxA.y)
          || boxA.width > viewport.width || boxB.width > viewport.width) {
        throw new Error('Comparison layout did not adapt to the viewport');
      }
      if (label === 'desktop' || label === 'small-phone') {
        await comparison.screenshot({ path: path.join(output, `${label}-comparison.png`) });
      }
      await playerA.evaluate(video => video.play());
      await playerB.evaluate(video => video.play());
      await comparison.locator('summary').click();
      await page.waitForFunction(() => [...document.querySelectorAll('[data-compare-video]')].every(video => video.paused));
    } else if (await comparison.count() !== 0) {
      throw new Error('Single-version delivery exposed an unusable comparison');
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
  if (await fallbackPage.locator('[data-review-compare]').count() !== 0) {
    throw new Error('Comparison controls appeared without JavaScript');
  }
  for (const [index, item] of delivered.entries()) {
    const section = fallbackPage.locator('section').nth(index);
    const notice = section.locator('noscript');
    if (!(await notice.isVisible()) || !(await notice.textContent()).includes(item.jobId)
      || !(await notice.textContent()).includes(`rendered revision ${item.revision}`)
      || !(await notice.textContent()).includes('start and end times')
      || await section.locator('[data-feedback-reference]').isVisible()
      || await section.locator('[data-feedback-range]').isVisible()
      || await section.locator('[data-feedback-draft]').isVisible()) {
      throw new Error('No-script feedback guidance is missing or interactive controls were exposed');
    }
  }
  await noScript.close();
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify({ passed: true,
    browser: await browser.version(), results, requests, noScriptFeedbackGuidance: true,
    offlineFeedbackDrafts: true, pointAndRangeDrafts: true, secureDraftIds: true,
    explicitApprovalAndChangesRequested: true, approvalWithoutComment: true,
    newDraftRequiresDeliberateDiscard: true, independentComparisonTested: delivered.length >= 2,
    draftCopyDownloadAndFallback: true, unicodeByteLimitEnforced: true, feedbackTextEscaped: true,
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
