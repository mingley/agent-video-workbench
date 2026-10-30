// Compare actual generated, queued renders through CLI and the official MCP SDK.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: compare_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const generated = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=8',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=8', '-c:v', 'libx264', '-preset', 'ultrafast',
  '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source], { encoding: 'utf8' });
if (generated.status !== 0) throw new Error(generated.stderr);
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const originalSha = sha(source);
const client = new Client({ name: 'avw-frozen-comparison', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
const ledger = [];
try {
  await client.connect(transport);
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ request, response: value });
    fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2));
    if (value.ok !== expectOk) throw new Error(JSON.stringify(value));
    return expectOk ? value.result : value.error;
  }
  await call({ command: 'create', project: 'project', name: 'Frozen source comparison' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'import-source' });
  const edit = { name: 'Baseline', outputId: 'baseline', quality: 'lossless', cuts: [
    { id: 'first', assetId: 'source', startMs: 0, endMs: 2000 },
    { id: 'second', assetId: 'source', startMs: 4000, endMs: 6000 },
  ] };
  await call({ command: 'assemble', project: 'project', expectedRevision: 1, key: 'baseline-edit', edit });
  const baseline = await call({ command: 'render-start', project: 'project', sequence: 'baseline',
    expectedRevision: 2, key: 'baseline-render', noLaunch: true });
  const candidateEdit = { ...edit, name: 'Candidate', outputId: 'candidate', allowReorder: true, cuts: [
    { id: 'second', assetId: 'source', startMs: 4000, endMs: 6000 },
    { id: 'first', assetId: 'source', startMs: 1000, endMs: 3000 },
  ] };
  await call({ command: 'assemble', project: 'project', expectedRevision: 2, key: 'candidate-edit', edit: candidateEdit });
  const candidate = await call({ command: 'render-start', project: 'project', sequence: 'candidate',
    expectedRevision: 3, key: 'candidate-render', noLaunch: true });
  const project = await call({ command: 'status', project: 'project' });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: project.projectId,
    baseRevision: 3, idempotencyKey: 'later-rename', operations: [
      { id: 'rename', op: 'project.rename', params: { name: 'Edited after freezing' } },
    ] } });
  const worker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (worker.status !== 0) throw new Error(worker.stderr);
  for (const job of [baseline, candidate]) {
    if ((await call({ command: 'job-status', project: 'project', id: job.id })).state !== 'succeeded') {
      throw new Error('Frozen render failed');
    }
  }
  const request = { command: 'compare-outputs', project: 'project', baselineJob: baseline.id, candidateJob: candidate.id };
  const before = await call({ command: 'status', project: 'project' });
  const report = await call(request);
  const t = value => value.value * value.rate.denominator / value.rate.numerator;
  if (report.status !== 'complete' || report.baseline.revision !== 2 || report.candidate.revision !== 3
    || report.baseline.sequenceBinding !== 'job-input' || report.baseline.review.role !== 'sdr-review-preview'
    || report.retained.length !== 2 || !report.retained.every(part => part.reordered === true)
    || report.added.length !== 1 || report.omitted.length !== 1
    || t(report.omitted[0].sourceStart) !== 0 || t(report.omitted[0].sourceEnd) !== 1
    || t(report.added[0].sourceStart) !== 2 || t(report.added[0].sourceEnd) !== 3
    || t(report.retained[0].baselinePlacements[0].outputStart) !== 1
    || t(report.retained[0].candidatePlacements[0].outputStart) !== 2) {
    throw new Error('Comparison did not preserve frozen source/output ranges: ' + JSON.stringify(report));
  }
  for (const side of [report.baseline, report.candidate]) {
    if (sha(side.master.path) !== side.master.sha256 || sha(side.review.path) !== side.review.sha256) {
      throw new Error('Comparison artifact hashes do not match the files');
    }
  }
  const requestFile = path.join(root, 'compare-request.json');
  fs.writeFileSync(requestFile, JSON.stringify(request));
  for (const args of [['request', requestFile], ['compare-outputs', 'project', baseline.id, candidate.id]]) {
    const cli = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root, ...args], { encoding: 'utf8' });
    if (cli.status !== 0 || JSON.stringify(JSON.parse(cli.stdout).result) !== JSON.stringify(report)) {
      throw new Error('CLI and MCP comparison results differ: ' + cli.stderr);
    }
  }
  const preview = report.candidate.review.path;
  const previewBytes = fs.readFileSync(preview);
  try {
    fs.writeFileSync(preview, 'tampered');
    const error = await call(request, false);
    if (!error.message.includes('preview bytes changed')) throw new Error('Tampered preview was not rejected');
  } finally { fs.writeFileSync(preview, previewBytes); }
  const master = report.baseline.master.path;
  const masterBytes = fs.readFileSync(master);
  try {
    fs.writeFileSync(master, 'tampered');
    const error = await call(request, false);
    if (!error.message.includes('master bytes changed')) throw new Error('Tampered master was not rejected');
  } finally { fs.writeFileSync(master, masterBytes); }
  if (JSON.stringify(before) !== JSON.stringify(await call({ command: 'status', project: 'project' }))
    || sha(source) !== originalSha) throw new Error('Comparison changed the project or original');
  const summary = { passed: true, officialMcpSdk: true, cliNamedAndJsonEquivalent: true,
    frozenRevisions: [report.baseline.revision, report.candidate.revision], currentRevision: before.revision,
    retained: report.retained.length, added: report.added.length, omitted: report.omitted.length,
    relativeReorderDetected: true, reviewPreviewAndMasterVerified: true,
    masterAndPreviewTamperingRejected: true, sourceUnchanged: true };
  fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify(summary, null, 2));
  console.log('Frozen output comparison passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
