// Exercise research-backed search, chapter drafting and delivery checks with
// generated media through the official MCP SDK and both CLI request forms.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: creator_research_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const generated = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=160x90:r=10:d=32',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=32', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source], { encoding: 'utf8' });
if (generated.status !== 0) throw new Error(generated.stderr);
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sourceHash = sha(source);
const client = new Client({ name: 'avw-creator-research', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
const ledger = [];
const jsonFile = (name, value) => {
  const file = path.join(root, name);
  fs.writeFileSync(file, JSON.stringify(value, null, 2));
  return file;
};
const cli = (args, probe = ffprobe) => {
  const run = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', probe, '--workspace', root, ...args],
    { encoding: 'utf8', timeout: 120000 });
  if (run.status !== 0) throw new Error(`CLI failed: ${run.stderr} ${run.stdout}`);
  return JSON.parse(run.stdout).result;
};
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
  const capabilities = await call({ command: 'capabilities' });
  if (!capabilities.commands.includes('delivery-check') || !capabilities.commands.includes('chapter-draft')) {
    throw new Error('Research-backed commands are missing from capabilities');
  }
  await call({ command: 'create', project: 'project', name: 'Research workflow' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'source-import' });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 1, key: 'unicode-transcript',
    transcript: { assetId: 'source', language: 'ja', provider: 'generated-test', cues: [
      { id: 'a', startMs: 0, endMs: 500, text: 'Cafe\u0301' },
      { id: 'b', startMs: 500, endMs: 1000, text: '世界' },
    ] } });
  const search = { command: 'transcript-search', project: 'project', assetId: 'source', query: 'Café世界',
    acrossCues: true, canonicalUnicode: true, joinWithoutSpaces: true, expectedRevision: 2 };
  const found = await call(search);
  if (found.items.length !== 1 || JSON.stringify(found.items[0].matchedCueIds) !== JSON.stringify(['a', 'b'])
      || !found.items[0].previewRequest) throw new Error('Unicode cue search failed: ' + JSON.stringify(found));
  const defaultSearch = await call({ ...search, canonicalUnicode: false, joinWithoutSpaces: false });
  if (defaultSearch.items.length !== 0) throw new Error('Default search behavior changed');
  if (JSON.stringify(cli(['transcript-search', 'project', 'Café世界', '--asset-id', 'source', '--across-cues',
      '--canonical-unicode', '--join-without-spaces', '--expected-revision', '2'])) !== JSON.stringify(found)) {
    throw new Error('Named CLI and MCP search results differ');
  }
  await call({ command: 'assemble', project: 'project', expectedRevision: 2, key: 'source-assembly',
    edit: { outputId: 'baseline', name: 'Baseline', cuts: [
      { id: 'whole', assetId: 'source', startMs: 0, endMs: 32000 },
    ] } });
  const project = await call({ command: 'status', project: 'project' });
  const at = value => ({ value, rate: { numerator: 10, denominator: 1 } });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: project.projectId,
    baseRevision: 3, idempotencyKey: 'private-markers', operations: [
      { id: 'one', op: 'marker.add', params: { id: 'm0', sequence: 'baseline', at: at(0), name: 'private zero' } },
      { id: 'two', op: 'marker.add', params: { id: 'm1', sequence: 'baseline', at: at(102), comment: 'private note' } },
      { id: 'three', op: 'marker.add', params: { id: 'm2', sequence: 'baseline', at: at(205) } },
      { id: 'private', op: 'marker.add', params: { id: 'secret', sequence: 'baseline', at: at(250), name: 'secret marker' } },
    ] } });
  const job = await call({ command: 'render-start', project: 'project', sequence: 'baseline',
    expectedRevision: 4, key: 'frozen-render', noLaunch: true });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: project.projectId,
    baseRevision: 4, idempotencyKey: 'later-rename', operations: [
      { id: 'rename', op: 'project.rename', params: { name: 'Later project' } },
    ] } });
  const worker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (worker.status !== 0) throw new Error(worker.stderr);
  const completed = await call({ command: 'job-status', project: 'project', id: job.id });
  if (completed.state !== 'succeeded') throw new Error(JSON.stringify(completed));
  const selections = [{ markerId: 'm0', title: 'Introduction' }, { markerId: 'm1', title: 'How it works' },
    { markerId: 'm2', title: 'Next steps' }];
  const chapterRequest = { command: 'chapter-draft', project: 'project', id: job.id, selections };
  const chapters = await call(chapterRequest);
  if (chapters.render.revision !== 4 || chapters.youtube.documentedTimestampChecks !== 'pass'
      || chapters.youtube.accountAndVideoEligibility !== 'unknown' || chapters.text !==
      '00:00 Introduction\n00:10 How it works\n00:20 Next steps' || !chapters.rounding.anyRounded
      || JSON.stringify(chapters).includes('private note') || JSON.stringify(chapters).includes('secret marker')) {
    throw new Error('Frozen chapter draft or privacy failed: ' + JSON.stringify(chapters));
  }
  const selectionFile = jsonFile('chapter-selections.json', selections);
  const chapterEnvelope = jsonFile('chapter-request.json', chapterRequest);
  for (const args of [['chapter-draft', 'project', job.id, '--request', selectionFile], ['request', chapterEnvelope]]) {
    if (JSON.stringify(cli(args)) !== JSON.stringify(chapters)) throw new Error('CLI/MCP chapter drafts differ');
  }
  const badChapters = await call({ ...chapterRequest, selections: [selections[1], selections[0]] });
  if (badChapters.youtube.documentedTimestampChecks !== 'fail') throw new Error('Bad chapter order was accepted');
  const brief = { schemaVersion: 1, dimensions: { width: 160, height: 90 }, aspectRatio: { width: 16, height: 9 },
    maxDurationMs: 32000, maxBytes: 100000000, color: 'sdr' };
  const checkRequest = { command: 'delivery-check', project: 'project', id: job.id, brief };
  const check = await call(checkRequest);
  if (check.status !== 'pass' || check.artifact.revision !== 4 || check.checks.length !== 5
      || !check.checks.every(item => item.status === 'pass') || sha(check.artifact.path) !== check.artifact.sha256) {
    throw new Error('Measured delivery check failed: ' + JSON.stringify(check));
  }
  const briefFile = jsonFile('delivery-brief.json', brief);
  const checkEnvelope = jsonFile('delivery-request.json', checkRequest);
  for (const args of [['delivery-check', 'project', job.id, '--request', briefFile], ['request', checkEnvelope]]) {
    if (JSON.stringify(cli(args)) !== JSON.stringify(check)) throw new Error('CLI/MCP delivery checks differ');
  }
  const failed = await call({ ...checkRequest, brief: { schemaVersion: 1, maxBytes: 1, requiredSidecars: ['vtt'] } });
  if (failed.status !== 'fail' || failed.checks.some(item => item.status !== 'fail')) {
    throw new Error('Failed size and missing-caption checks were not reported');
  }
  const unknown = cli(['delivery-check', 'project', job.id, '--request', briefFile], '/nonexistent/ffprobe');
  if (unknown.status !== 'unknown' || !unknown.checks.some(item => item.status === 'unknown')
      || !unknown.checks.some(item => item.constraint === 'maxBytes' && item.status === 'pass')) {
    throw new Error('Unavailable probe did not preserve measured byte evidence');
  }
  const master = check.artifact.path;
  const bytes = fs.readFileSync(master);
  try {
    fs.writeFileSync(master, 'tampered');
    const error = await call(checkRequest, false);
    if (!error.message.includes('bytes changed')) throw new Error('Tampered master was not rejected');
  } finally { fs.writeFileSync(master, bytes); }
  if ((await call({ command: 'status', project: 'project' })).revision !== 5 || sha(source) !== sourceHash) {
    throw new Error('Read-only checks changed project history or source bytes');
  }
  const summary = { passed: true, officialMcpSdk: true, namedCliAndJsonEquivalent: true,
    unicodeAndNoSpaceSearch: true, frozenChapterRevision: chapters.render.revision, chapterOrderFailure: true,
    selectedMarkersOnly: true, deliveryPassFailUnknown: true, tamperedMasterRejected: true, sourceUnchanged: true };
  jsonFile('summary.json', summary);
  console.log('Creator research workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
