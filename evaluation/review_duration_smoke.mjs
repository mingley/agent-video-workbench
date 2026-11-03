// Check review bounds against a real render with an explicit sequence duration.
import { chromium } from 'playwright';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const [avw, ffmpeg, ffprobe, chromiumPath, output] = process.argv.slice(2);
if (!output) throw new Error('Usage: review_duration_smoke.mjs AVW FFMPEG FFPROBE CHROMIUM NEW_ROOT');
const root = path.resolve(output);
fs.mkdirSync(root, { recursive: false });
const project = path.join(root, 'project');
const source = path.join(root, 'source.mp4');
const ledger = [];
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const writeLedger = () => fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2) + '\n');
function run(argv) {
  const result = spawnSync(argv[0], argv.slice(1), { encoding: 'utf8', timeout: 180000 });
  ledger.push({ argv, status: result.status, stderr: result.stderr });
  writeLedger();
  if (result.status !== 0) throw new Error(result.stderr || result.error || 'Command failed');
  return result.stdout;
}
function call(command, fields = {}, expectOk = true) {
  const request = { command, project, ...fields };
  const argv = [avw, '--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'request', '-'];
  const result = spawnSync(argv[0], argv.slice(1), {
    input: JSON.stringify(request), encoding: 'utf8', timeout: 180000,
  });
  let response;
  try { response = JSON.parse(result.stdout); } catch { throw new Error(result.stderr || result.stdout); }
  ledger.push({ request, response, status: result.status });
  writeLedger();
  if (response.ok !== expectOk || (result.status === 0) !== expectOk) throw new Error(JSON.stringify(response));
  return expectOk ? response.result : response.error;
}
run([ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=4',
  '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv', source]);
const sourceSha = sha(source);
call('create', { name: 'Generated review timing regression' });
call('import', { source, id: 'source', expectedRevision: 0, key: 'import-source' });
const state = call('status');
const rate = { numerator: 30, denominator: 1 };
const time = value => ({ value, rate });
call('apply', { batch: { schemaVersion: '1.0.0', projectId: state.projectId, baseRevision: 1,
  idempotencyKey: 'review-timing-sequence', operations: [
    { id: 'seq', op: 'sequence.add', params: { id: 'extended', name: 'Explicit duration', width: 320, height: 180, frameRate: rate } },
    { id: 'track', op: 'track.add', params: { id: 'video', sequence: 'extended', type: 'video' } },
    { id: 'clip', op: 'clip.add', params: { id: 'take', asset: 'source', track: 'video', at: time(0), sourceIn: time(0), duration: time(120), fit: 'contain' } },
    { id: 'duration', op: 'sequence.set', target: 'extended', params: { duration: time(180) } },
  ] } });
// This synchronous artifact also checks that previously supported review jobs remain usable.
const render = call('render', { sequence: 'extended' });
const manifestBytes = fs.readFileSync(render.manifest);
const manifest = JSON.parse(manifestBytes);
const stream = manifest.verification.probe.streams.find(item => item.codec_type === 'video');
if (manifest.verification.expectedFrames !== 180 || stream.duration_ts * Number(stream.time_base.split('/')[0])
    !== 6 * Number(stream.time_base.split('/')[1])) throw new Error('Unexpected generated stream duration');
const artifact = call('artifact', { id: render.artifactId });
const masterSha = sha(render.path);
if (!artifact.verified || artifact.sha256 !== masterSha) throw new Error('Generated master was not verified');
const validDestination = path.join(root, 'review-valid');
call('delivery', { id: render.artifactId, destination: validDestination });
const validPage = fs.readFileSync(path.join(validDestination, 'index.html'), 'utf8');
if (!validPage.includes("data-review-limit-ms='6000'")) throw new Error('Review page used content end instead of actual duration');
const review = { action: 'review-add', id: 'ending', jobId: render.artifactId,
  atMs: 4500, endMs: 5000, actor: 'creator', text: 'Feedback on the ending' };
const accepted = call('studio', { expectedRevision: 2, key: 'review-ending', edit: review });
const beyond = call('studio', { expectedRevision: 3, key: 'review-beyond',
  edit: { ...review, id: 'beyond', atMs: 5999, endMs: 6001 } }, false);
if (!beyond.message.includes('beyond artifact duration')) throw new Error('Out-of-bounds review was accepted');
const before = call('status');
const history = call('history');
let missingPage;
try {
  // Simulate a historical manifest with no usable probe without changing media.
  manifest.verification.probe = null;
  fs.writeFileSync(render.manifest, JSON.stringify(manifest, null, 2));
  const destination = path.join(root, 'review-unknown');
  call('delivery', { id: render.artifactId, destination });
  missingPage = path.join(destination, 'index.html');
  const html = fs.readFileSync(missingPage, 'utf8');
  if (!html.includes("data-review-limit-ms=''") || !html.includes('data-range-unavailable')) {
    throw new Error('Unknown timing did not produce a visible notice');
  }
  const missing = call('studio', { expectedRevision: 3, key: 'review-unknown',
    edit: { ...review, id: 'unknown', atMs: 1000, endMs: 2000 } }, false);
  if (!missing.message.includes('re-render')) throw new Error('Unknown review timing was accepted');
  const replay = call('studio', { expectedRevision: 2, key: 'review-ending', edit: review });
  if (JSON.stringify(replay) !== JSON.stringify(accepted)) throw new Error('Committed review did not replay');
  if (JSON.stringify(call('status')) !== JSON.stringify(before)
      || JSON.stringify(call('history')) !== JSON.stringify(history)) throw new Error('Refused review changed history');
} finally {
  fs.writeFileSync(render.manifest, manifestBytes);
}
if (sha(render.path) !== masterSha || sha(source) !== sourceSha
    || !fs.readFileSync(render.manifest).equals(manifestBytes)) throw new Error('Source or artifact was changed');

// Serve only the two review packages, including byte ranges for video seeking.
const packages = { valid: fs.realpathSync(validDestination), unknown: fs.realpathSync(path.dirname(missingPage)) };
const server = http.createServer((request, response) => {
  try {
    const parts = decodeURIComponent(new URL(request.url, 'http://localhost').pathname).split('/');
    const packageRoot = packages[parts[1]];
    if (!packageRoot || parts.length < 3) { response.writeHead(404); response.end(); return; }
    const file = fs.realpathSync(path.resolve(packageRoot, ...parts.slice(2)));
    if (!file.startsWith(packageRoot + path.sep) || !fs.statSync(file).isFile()) {
      response.writeHead(403); response.end(); return;
    }
    const size = fs.statSync(file).size;
    const headers = { 'Content-Type': ({ '.html': 'text/html', '.mp4': 'video/mp4' })[path.extname(file)] ?? 'application/octet-stream',
      'Accept-Ranges': 'bytes' };
    let start = 0, end = size - 1, status = 200;
    if (request.headers.range) {
      const match = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range);
      if (!match) { response.writeHead(416); response.end(); return; }
      start = Number(match[1]); end = match[2] ? Number(match[2]) : end;
      if (start >= size || end >= size || start > end) { response.writeHead(416); response.end(); return; }
      headers['Content-Range'] = `bytes ${start}-${end}/${size}`;
      status = 206;
    }
    headers['Content-Length'] = end - start + 1;
    response.writeHead(status, headers);
    if (request.method === 'HEAD' || size === 0) response.end();
    else fs.createReadStream(file, { start, end }).on('error', () => response.destroy()).pipe(response);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = 'http://127.0.0.1:' + server.address().port;
let browser;
try {
  // Inspect actual browser behavior for both known and unknown timing.
  browser = await chromium.launch({ executablePath: chromiumPath, headless: true, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.route('**/*', route => route.request().url().startsWith(base + '/') ? route.continue() : route.abort());
  for (const [packageName, known] of [['valid', true], ['unknown', false]]) {
    await page.goto(base + '/' + packageName + '/index.html');
    const video = page.locator('video');
    await video.evaluate(async element => {
      if (element.readyState < 1) await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('metadata timed out')), 5000);
        element.addEventListener('loadedmetadata', () => { clearTimeout(timer); resolve(); }, { once: true });
      });
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('seek timed out')), 5000);
        element.addEventListener('seeked', () => { clearTimeout(timer); resolve(); }, { once: true });
        element.currentTime = 4.5;
      });
    });
    if (!(await page.locator('[data-feedback-reference]').isVisible())
        || !(await page.locator('[data-feedback-reference] textarea').inputValue()).includes('4500 ms')) {
      throw new Error('Point feedback controls are not available at the played time');
    }
    if ((await page.locator('[data-feedback-range]').isVisible()) !== known
        || (await page.locator('[data-range-unavailable]').count()) !== (known ? 0 : 1)) {
      throw new Error('Range controls do not reflect the available timing evidence');
    }
  }
} finally {
  if (browser) await browser.close();
  await new Promise(resolve => server.close(resolve));
}
const summary = { passed: true, generatedSourceSeconds: 4, frozenRenderedSeconds: 6,
  verifiedFrames: 180, reviewLimitMs: 6000, acceptedRangeMs: [4500, 5000],
  beyondEndRejected: true, missingTimingNotice: true, missingTimingReviewRejected: true,
  originalReviewReplayed: true, projectHistoryUnchangedOnRefusal: true,
  browserPointReferenceAvailable: true, browserRangeWithheldWhenUnknown: true,
  sourceAndArtifactUnchanged: true };
fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify(summary, null, 2) + '\n');
console.log('Review duration regression passed:', path.join(root, 'summary.json'));
