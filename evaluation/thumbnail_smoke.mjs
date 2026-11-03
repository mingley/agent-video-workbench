// Select thumbnail candidates from real frozen renders through CLI and MCP.
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: thumbnail_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const generated = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=160x90:r=30000/1001:d=3',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source], { encoding: 'utf8', timeout: 120000 });
if (generated.status !== 0) throw new Error(generated.stderr);
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sourceHash = sha(source);
const ledger = [];
const client = new Client({ name: 'avw-thumbnail-selection', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
function writeJson(name, value) {
  const file = path.join(root, name);
  fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
  return file;
}
function cli(args) {
  const run = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root, ...args],
    { encoding: 'utf8', timeout: 120000 });
  if (run.status !== 0) throw new Error(`CLI failed: ${run.stderr} ${run.stdout}`);
  return JSON.parse(run.stdout).result;
}
function decode(args) {
  const run = spawnSync(ffmpeg, ['-v', 'error', '-xerror', ...args, '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
    { timeout: 120000, maxBuffer: 16 * 1024 * 1024 });
  if (run.status !== 0) throw new Error(run.stderr.toString());
  return run.stdout;
}
try {
  await client.connect(transport);
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ request, response: value });
    writeJson('commands.json', ledger);
    if (value.ok !== expectOk) throw new Error(JSON.stringify(value));
    return expectOk ? value.result : value.error;
  }
  await call({ command: 'create', project: 'project', name: 'Generated thumbnail candidates' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'thumbnail-import-source' });
  await call({ command: 'assemble', project: 'project', expectedRevision: 1, key: 'thumbnail-assembly',
    edit: { outputId: 'candidate', name: 'Candidate', quality: 'lossless', cuts: [
      { id: 'cut', assetId: 'source', startMs: 0, endMs: 2000 },
    ] } });
  const job = await call({ command: 'render-start', project: 'project', sequence: 'candidate',
    expectedRevision: 2, key: 'thumbnail-render', noLaunch: true });
  const project = await call({ command: 'status', project: 'project' });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: project.projectId,
    baseRevision: 2, idempotencyKey: 'thumbnail-later-rename', operations: [
      { id: 'rename', op: 'project.rename', params: { name: 'Later project' } },
    ] } });
  const worker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (worker.status !== 0) throw new Error(worker.stderr);
  const state = await call({ command: 'job-status', project: 'project', id: job.id });
  if (state.state !== 'succeeded') throw new Error(JSON.stringify(state));
  const master = await call({ command: 'artifact', project: 'project', id: job.id });
  const preview = await call({ command: 'artifact', project: 'project', id: job.id, preview: true });
  if (preview.role !== 'sdr-review-preview' || !preview.verified || !master.verified) throw new Error('Preview missing');
  const masterHash = sha(master.path);
  const previewHash = sha(preview.path);
  const selection = { schemaVersion: 1, atMs: [1499, 0, 501] };
  const request = { command: 'thumbnail-candidates', project: 'project', id: job.id,
    destination: 'thumbnails-mcp', selection };
  const result = await call(request);
  const manifest = result.manifest;
  if (manifest.revision !== 2 || manifest.source.role !== 'sdr-review-preview'
      || manifest.source.sha256 !== previewHash || manifest.source.masterSha256 !== masterHash
      || JSON.stringify(manifest.candidates.map(item => item.requestedAtMs)) !== JSON.stringify(selection.atMs)
      || JSON.stringify(manifest.candidates.map(item => item.frameIndex)) !== JSON.stringify([44, 0, 15])
      || JSON.stringify(JSON.parse(fs.readFileSync(path.join(result.path, 'manifest.json')))) !== JSON.stringify(manifest)
      || fs.existsSync(path.join(result.path, 'thumbnails.incomplete'))) {
    throw new Error('Thumbnail manifest lost frozen timing or source identity: ' + JSON.stringify(manifest));
  }
  for (const candidate of manifest.candidates) {
    const actual = path.join(result.path, candidate.path);
    if (candidate.image.mimeType !== 'image/png' || sha(actual) !== candidate.image.sha256) {
      throw new Error('Thumbnail PNG hash is wrong');
    }
    const seconds = candidate.decodedTimestamp.value * candidate.decodedTimestamp.rate.denominator
      / candidate.decodedTimestamp.rate.numerator;
    if (Math.abs(seconds - candidate.frameIndex * 1001 / 30000) > 0.0000001) throw new Error('Decoded frame timestamp changed');
    const png = decode(['-i', actual, '-frames:v', '1']);
    const reference = decode(['-i', preview.path, '-vf', `select=eq(n\\,${candidate.frameIndex}),scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease`,
      '-frames:v', '1']);
    if (!png.equals(reference)) throw new Error('Selected PNG differs from the decoded reference frame');
  }
  const selectionFile = writeJson('selection.json', selection);
  const cliResult = cli(['thumbnail-candidates', 'project', job.id, 'thumbnails-cli', '--request', selectionFile]);
  const jsonRequest = writeJson('request.json', { ...request, destination: 'thumbnails-json' });
  const jsonResult = cli(['request', jsonRequest]);
  if (JSON.stringify(cliResult.manifest) !== JSON.stringify(manifest)
      || JSON.stringify(jsonResult.manifest) !== JSON.stringify(manifest)) throw new Error('CLI/MCP thumbnail manifests differ');
  const first = path.join(result.path, manifest.candidates[0].path);
  const firstHash = sha(first);
  const existing = await call(request, false);
  if (!existing.message.includes('already exists') || sha(first) !== firstHash) throw new Error('Existing destination was overwritten');
  const invalid = await call({ ...request, destination: 'invalid', selection: { schemaVersion: 1, atMs: [2003] } }, false);
  if (!invalid.message.includes('outside') || fs.existsSync(path.join(root, 'invalid'))) throw new Error('Out-of-range timestamp was accepted');
  const duplicate = await call({ ...request, destination: 'duplicate', selection: { schemaVersion: 1, atMs: [0, 1] } }, false);
  if (!duplicate.message.includes('distinct') || fs.existsSync(path.join(root, 'duplicate'))) throw new Error('Duplicate output frame was accepted');
  const previewBytes = fs.readFileSync(preview.path);
  try {
    fs.writeFileSync(preview.path, 'tampered');
    const error = await call({ ...request, destination: 'tampered' }, false);
    if (!error.message.includes('bytes changed') || fs.existsSync(path.join(root, 'tampered'))) throw new Error('Changed preview was accepted');
  } finally { fs.writeFileSync(preview.path, previewBytes); }
  if ((await call({ command: 'status', project: 'project' })).revision !== 3
      || sha(source) !== sourceHash || sha(master.path) !== masterHash || sha(preview.path) !== previewHash) {
    throw new Error('Thumbnail extraction changed a project or source artifact');
  }
  writeJson('summary.json', { passed: true, officialMcpSdk: true, namedCliAndJsonEquivalent: true,
    frozenRevision: 2, currentRevision: 3, sourceRole: preview.role, candidates: 3,
    decodedPngMatchesReferenceFrames: true, exactFractionalRateDecodedPtsChecked: true, sourceAndRenderUnchanged: true,
    outOfRangeAndDuplicateFramesRejected: true, changedPreviewRejected: true, destinationNotOverwritten: true });
  console.log('Thumbnail candidates passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
