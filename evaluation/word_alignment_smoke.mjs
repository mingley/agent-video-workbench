// Exercise source-bound word alignment through the official MCP SDK and CLI.
// The media and transcript are synthetic fixtures, not an ASR accuracy test.
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [avw, ffmpeg, ffprobe, font, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) {
  throw new Error('Usage: word_alignment_smoke.mjs AVW FFMPEG FFPROBE FONT NEW_ROOT SDK_ROOT');
}
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const ledger = [];
const writeLedger = () => fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2) + '\n');
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const generated = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=3',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source], { encoding: 'utf8', timeout: 120000 });
ledger.push({ transport: 'fixture', command: 'generate test video and tone', status: generated.status, stderr: generated.stderr });
writeLedger();
if (generated.status !== 0) throw new Error(generated.stderr);
fs.copyFileSync(font, path.join(root, 'font.ttf'));
const sourceSha = sha(source);
const client = new Client({ name: 'avw-word-alignment-conformance', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
const jsonFile = (name, value) => {
  const file = path.join(root, name);
  fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
  return file;
};
const cli = (args, expectOk = true) => {
  const result = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root, ...args],
    { encoding: 'utf8', timeout: 120000 });
  let value;
  try { value = JSON.parse(result.stdout); } catch { throw new Error(`CLI returned non-JSON: ${result.stderr} ${result.stdout}`); }
  ledger.push({ transport: 'cli', args, response: value });
  writeLedger();
  if (value.ok !== expectOk || (result.status === 0) !== expectOk) throw new Error(JSON.stringify(value));
  return expectOk ? value.result : value.error;
};
const same = (a, b, message) => {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(message + ': ' + JSON.stringify({ a, b }));
};

try {
  await client.connect(transport);
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ transport: 'mcp', request, response: value });
    writeLedger();
    if (value.ok !== expectOk) throw new Error(JSON.stringify(value));
    return expectOk ? value.result : value.error;
  }
  const tools = await client.listTools();
  if (!JSON.stringify(tools).includes('preview-word-alignment') || !JSON.stringify(tools).includes('accept-word-alignment')) {
    throw new Error('Word alignment operations are missing from the MCP schema');
  }
  await call({ command: 'create', project: 'project', name: 'Synthetic word alignment' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'import-source' });
  await call({ command: 'import', project: 'project', source: 'font.ttf', id: 'font',
    expectedRevision: 1, key: 'import-font' });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 2, key: 'synthetic-transcript',
    transcript: { assetId: 'source', language: 'en', provider: 'synthetic-fixture', cues: [
      { id: 'c1', startMs: 200, endMs: 1000, text: 'Cafe cafe!' },
      { id: 'c2', startMs: 1100, endMs: 1900, text: 'Go go!' },
    ] } });
  await call({ command: 'studio', project: 'project', expectedRevision: 3, key: 'correct-utf8-transcript',
    edit: { action: 'transcript-correct', assetId: 'source', cues: { c1: 'Café café!' }, sequences: [] } });
  await call({ command: 'compose', project: 'project', expectedRevision: 4, key: 'compose-captions',
    edit: { outputId: 'captioned', name: 'Captioned synthetic fixture', fontAssetId: 'font',
      width: 360, height: 640, fontSize: 20,
      cuts: [{ id: 'take', assetId: 'source', startMs: 0, endMs: 2500 }] } });
  const before = await call({ command: 'status', project: 'project' });
  if (before.revision !== 5) throw new Error('Unexpected project revision');
  const selected = before.extensions['avw.transcripts.v1'].find(item => item.assetId === 'source');
  if (selected.cues[0].text !== 'Café café!') throw new Error('UTF-8 correction was not selected');
  const imported = before.assets.find(asset => asset.id === 'source');
  const importedFile = path.join(root, 'project', imported.uri);
  if (imported.fingerprint.sha256 !== sourceSha || sha(importedFile) !== sourceSha) {
    throw new Error('Imported source identity does not match original bytes');
  }
  const originalRender = await call({ command: 'render', project: 'project', sequence: 'captioned' });
  const priorSrt = path.join(path.dirname(originalRender.path), 'captions.srt');
  const priorVtt = path.join(path.dirname(originalRender.path), 'captions.vtt');
  const priorArtifacts = [originalRender.path, originalRender.manifest, priorSrt, priorVtt];
  const priorHashes = priorArtifacts.map(sha);
  if (!fs.readFileSync(priorSrt, 'utf8').includes('Café café!')) throw new Error('Corrected caption was not rendered');
  const search = await call({ command: 'transcript-search', project: 'project', assetId: 'source',
    query: 'Café', acrossCues: true, expectedRevision: 5 });
  if (search.items.length !== 1 || search.items[0].matchedCueIds[0] !== 'c1') {
    throw new Error('Corrected transcript could not be located');
  }
  const transcriptSha = search.items[0].transcript.sha256;
  const proposal = { schemaVersion: 1, assetId: 'source', sourceSha256: sourceSha,
    transcriptSha256: transcriptSha, provider: 'synthetic-fixture', providerVersion: '1', words: [
      { id: 'w1', cueId: 'c1', text: 'Café', textStartByte: 0, textEndByte: 5, startMs: 200, endMs: 500,
        speaker: 'Speaker 1', confidence: 0.8 },
      { id: 'w2', cueId: 'c1', text: 'café!', textStartByte: 6, textEndByte: 12, startMs: 550, endMs: 1000,
        speaker: 'Speaker 1', confidence: null },
      { id: 'w3', cueId: 'c2', text: 'Go', textStartByte: 0, textEndByte: 2, startMs: 1100, endMs: 1400,
        speaker: 'Speaker 1', confidence: null },
      { id: 'w4', cueId: 'c2', text: 'go!', textStartByte: 3, textEndByte: 6, startMs: 1450, endMs: 1900,
        speaker: 'Speaker 1', confidence: null },
    ] };
  const review = { reviewer: 'fixture', note: 'Synthetic tone and manually supplied timestamps; no speech accuracy claim.' };
  const proposalFile = jsonFile('alignment-proposal.json', proposal);
  const reviewFile = jsonFile('alignment-review.json', review);
  const previewRequest = { command: 'preview-word-alignment', project: 'project', expectedRevision: 5, proposal };
  const previewFile = jsonFile('preview-envelope.json', previewRequest);
  const preview = await call(previewRequest);
  if (!preview.sourceBytesVerified || preview.transcriptSha256 !== transcriptSha
      || preview.wordCount !== 4 || preview.selectedCueCount !== 2 || preview.unalignedCueCount !== 0) {
    throw new Error('Preview lost source, transcript, or word evidence: ' + JSON.stringify(preview));
  }
  same(cli(['preview-word-alignment', 'project', '--expected-revision', '5', '--request', proposalFile]), preview,
    'Named CLI and MCP preview differ');
  same(cli(['request', previewFile]), preview, 'JSON CLI and MCP preview differ');
  const invalid = structuredClone(previewRequest);
  invalid.proposal.words[0].textEndByte = 4; // Splits the two-byte é.
  const rejected = await call(invalid, false);
  if (rejected.code !== 'E_INVALID_REQUEST' || !rejected.message.includes('UTF-8')) {
    throw new Error('Split UTF-8 boundary was accepted: ' + JSON.stringify(rejected));
  }
  const historyBefore = await call({ command: 'history', project: 'project' });
  const acceptRequest = { command: 'accept-word-alignment', project: 'project', expectedRevision: 5,
    key: 'accept-reviewed-words', proposal, review };
  const namedAccept = ['accept-word-alignment', 'project', '--expected-revision', '5', '--key', acceptRequest.key,
    '--request', proposalFile, '--review', reviewFile];
  const dry = await call({ ...acceptRequest, dryRun: true });
  same(cli([...namedAccept, '--dry-run']), dry, 'Named CLI and MCP dry-runs differ');
  if (dry.previousRevision !== 5 || dry.revision !== 6
      || (await call({ command: 'status', project: 'project' })).revision !== 5) {
    throw new Error('Dry-run changed project history');
  }
  same(await call({ command: 'history', project: 'project' }), historyBefore, 'Dry-run changed history');
  const accepted = await call(acceptRequest);
  same(accepted, dry, 'Committed alignment differed from dry-run');
  same(cli(namedAccept), accepted, 'Named CLI replay differs from MCP acceptance');
  same(cli(['request', jsonFile('accept-envelope.json', acceptRequest)]), accepted, 'JSON CLI replay differs from MCP acceptance');
  const after = await call({ command: 'status', project: 'project' });
  if (after.revision !== 6) throw new Error('Alignment did not commit exactly one revision');
  same(after.sequences, before.sequences, 'Alignment changed captioned sequences');
  same(after.extensions['avw.transcripts.v1'], before.extensions['avw.transcripts.v1'], 'Alignment changed corrected transcript');
  same(after.extensions['avw.corrections.source'], before.extensions['avw.corrections.source'], 'Alignment changed correction history');
  const pageRequest = { command: 'word-alignment', project: 'project', assetId: 'source',
    expectedRevision: 6, offset: 0, limit: 2 };
  const first = await call(pageRequest);
  if (first.status !== 'current' || !first.sourceBytesVerified || first.wordCount !== 4
      || first.items[0].text !== 'Café' || first.items[1].text !== 'café!' || first.nextOffset !== 2) {
    throw new Error('First alignment page is incorrect: ' + JSON.stringify(first));
  }
  same(cli(['word-alignment', 'project', '--asset-id', 'source', '--expected-revision', '6', '--offset', '0', '--limit', '2']),
    first, 'Named CLI and MCP alignment page differ');
  same(cli(['request', jsonFile('page-envelope.json', pageRequest)]), first, 'JSON CLI and MCP alignment page differ');
  const second = await call({ ...pageRequest, offset: 2 });
  if (second.items.length !== 2 || second.items[0].text !== 'Go' || second.items[1].text !== 'go!'
      || second.nextOffset !== null) throw new Error('Alignment continuation failed');
  const noGuard = await call({ ...pageRequest, expectedRevision: undefined, offset: 2 }, false);
  if (noGuard.code !== 'E_INVALID_REQUEST' || !noGuard.message.includes('expectedRevision')) {
    throw new Error('Alignment continuation did not require a revision guard');
  }
  const nextRender = await call({ command: 'render', project: 'project', sequence: 'captioned' });
  const nextSrt = path.join(path.dirname(nextRender.path), 'captions.srt');
  const nextVtt = path.join(path.dirname(nextRender.path), 'captions.vtt');
  if (!fs.readFileSync(priorSrt).equals(fs.readFileSync(nextSrt))
      || !fs.readFileSync(priorVtt).equals(fs.readFileSync(nextVtt))) {
    throw new Error('Alignment changed generated caption sidecars');
  }
  const bytes = fs.readFileSync(importedFile);
  try {
    fs.writeFileSync(importedFile, 'tampered');
    const unavailable = await call({ ...pageRequest, offset: 0, limit: 2 });
    if (unavailable.status !== 'unavailable' || unavailable.sourceBytesVerified !== false) {
      throw new Error('Altered original remained current');
    }
    const invalidSource = await call({ ...previewRequest, expectedRevision: 6 }, false);
    if (!invalidSource.message.includes('bytes changed')) throw new Error('Altered source was not rejected');
  } finally { fs.writeFileSync(importedFile, bytes); }
  await call({ command: 'studio', project: 'project', expectedRevision: 6, key: 'later-transcript-correction',
    edit: { action: 'transcript-correct', assetId: 'source', cues: { c1: 'Café coffee!' }, sequences: [] } });
  const changed = await call({ command: 'status', project: 'project' });
  const stale = await call({ ...pageRequest, expectedRevision: 7, offset: 0, limit: 2 });
  if (stale.status !== 'stale' || !stale.staleReasons.includes('selected-transcript-changed')) {
    throw new Error('Changed transcript did not make alignment stale');
  }
  const stalePreview = await call({ ...previewRequest, expectedRevision: 7 }, false);
  if (!stalePreview.message.includes('stale')) throw new Error('Stale proposal was accepted');
  same(await call(acceptRequest), accepted, 'Earlier accepted request failed to replay');
  same(await call({ command: 'status', project: 'project' }), changed, 'Replay changed current project state');
  if (sha(source) !== sourceSha || sha(importedFile) !== sourceSha
      || JSON.stringify(priorArtifacts.map(sha)) !== JSON.stringify(priorHashes)) {
    throw new Error('Original source or historical captioned artifact changed');
  }
  const summary = { passed: true, officialMcpSdk: true, namedCliAndJsonEquivalent: true,
    generatedVideoAndTone: true, utf8CorrectedTranscript: true, exactTextByteSpans: true,
    invalidUtf8BoundaryRejected: true, dryRunUnchanged: true, acceptanceAndReplay: true,
    revisionGuardedPagination: true, sourceTamperUnavailable: true, transcriptCorrectionMarkedStale: true,
    sourceAndPriorArtifactsUnchanged: true, renderedSrtVttUnchanged: true,
    limitations: ['Synthetic timing and tone only; no speech or alignment accuracy claim'] };
  jsonFile('summary.json', summary);
  console.log('Word alignment workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
