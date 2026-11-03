// Exercise caller-supplied caption translations with real generated renders.
// The supplied text is a synthetic fixture, not a translation-accuracy test.
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { isDeepStrictEqual } from 'node:util';

const [avw, ffmpeg, ffprobe, font, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: translation_smoke.mjs AVW FFMPEG FFPROBE FONT NEW_ROOT SDK_ROOT');
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
const client = new Client({ name: 'avw-translation-conformance', version: '1.0.0' });
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
  if (!isDeepStrictEqual(a, b)) throw new Error(message + ': ' + JSON.stringify({ a, b }));
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
  if (!JSON.stringify(tools).includes('translation-context') || !JSON.stringify(tools).includes('plan-translation')) {
    throw new Error('Translation proposal operations are missing from the MCP schema');
  }
  await call({ command: 'create', project: 'project', name: 'Synthetic translation review' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'import-source' });
  await call({ command: 'import', project: 'project', source: 'font.ttf', id: 'font',
    expectedRevision: 1, key: 'import-font' });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 2, key: 'synthetic-transcript',
    transcript: { assetId: 'source', language: 'en', provider: 'synthetic-fixture', cues: [
      { id: 'c1', startMs: 200, endMs: 1000, text: 'Coffee demo' },
      { id: 'c2', startMs: 1100, endMs: 1900, text: 'Hello world' },
    ] } });
  await call({ command: 'compose', project: 'project', expectedRevision: 3, key: 'compose-captions',
    edit: { outputId: 'captioned', name: 'Original captioned output', fontAssetId: 'font',
      width: 360, height: 640, fontSize: 20,
      cuts: [{ id: 'take', assetId: 'source', startMs: 0, endMs: 2500 }] } });
  const before = await call({ command: 'status', project: 'project' });
  if (before.revision !== 4) throw new Error('Unexpected project revision');
  const imported = before.assets.find(asset => asset.id === 'source');
  const importedFile = path.join(root, 'project', imported.uri);
  if (sha(importedFile) !== sourceSha) throw new Error('Imported original differs from source');
  const original = await call({ command: 'render', project: 'project', sequence: 'captioned' });
  const originalSrt = path.join(path.dirname(original.path), 'captions.srt');
  const originalVtt = path.join(path.dirname(original.path), 'captions.vtt');
  const originalArtifacts = [original.path, original.manifest, originalSrt, originalVtt];
  const originalHashes = originalArtifacts.map(sha);
  if (!fs.readFileSync(originalSrt, 'utf8').includes('Coffee demo')) throw new Error('Original captions not present');
  const contextRequest = { command: 'translation-context', project: 'project', sequence: 'captioned',
    expectedRevision: 4, offset: 0, limit: 1 };
  const first = await call(contextRequest);
  same(cli(['translation-context', 'project', '--sequence', 'captioned', '--expected-revision', '4', '--offset', '0', '--limit', '1']),
    first, 'Named CLI and MCP translation context differ');
  same(cli(['request', jsonFile('context-envelope.json', contextRequest)]), first, 'JSON CLI and MCP translation context differ');
  const second = await call({ ...contextRequest, offset: 1 });
  if (first.captionCount !== 2 || first.visibleCaptionCount !== 2 || first.nextOffset !== 1
      || second.nextOffset !== null || first.sequenceSha256 !== second.sequenceSha256) {
    throw new Error('Caption context pagination or frozen sequence identity is incorrect');
  }
  const rows = [...first.items, ...second.items];
  if (rows.some(row => !row.visible || row.sourceCue.assetId !== 'source' || row.sourceAssetSha256 !== sourceSha)) {
    throw new Error('Caption source provenance is missing');
  }
  const translatedText = new Map([['Coffee demo', 'Café listo'], ['Hello world', 'Hola mundo']]);
  const proposal = { schemaVersion: 1, projectId: first.projectId, sourceRevision: 4, sequence: 'captioned',
    sequenceSha256: first.sequenceSha256, outputId: 'spanish', outputName: 'Spanish captions',
    targetLanguage: 'es', provider: 'synthetic-supplied-text', providerVersion: '1',
    translations: rows.map(row => ({ captionId: row.captionId, sourceTextSha256: row.textSha256,
      text: translatedText.get(row.text) })) };
  if (proposal.translations.some(item => !item.text)) throw new Error('Unexpected source caption fixture');
  const proposalFile = jsonFile('translation-proposal.json', proposal);
  const planRequest = { command: 'plan-translation', project: 'project', expectedRevision: 4,
    key: 'create-spanish-variant', proposal };
  const plan = await call(planRequest);
  if (plan.status !== 'ready' || !plan.providerProvenanceIncluded || plan.captionCount !== 2
      || plan.visibleCaptionCount !== 2 || plan.variantDryRun.revision !== 5
      || plan.studioRequest.dryRun !== true || plan.proposalSha256.length !== 64) {
    throw new Error('Translation plan did not bind a complete dry-run: ' + JSON.stringify(plan));
  }
  same(plan.proposal, proposal, 'Translation plan lost caller provenance');
  same(plan.studioRequest.edit.translationProvenance.proposal, proposal, 'Variant lost attributable proposal');
  same(cli(['plan-translation', 'project', '--expected-revision', '4', '--key', planRequest.key, '--request', proposalFile]),
    plan, 'Named CLI and MCP translation plans differ');
  same(cli(['request', jsonFile('plan-envelope.json', planRequest)]), plan, 'JSON CLI and MCP translation plans differ');
  const history = await call({ command: 'history', project: 'project' });
  if ((await call({ command: 'status', project: 'project' })).revision !== 4) throw new Error('Planning changed project revision');
  const actualDryRun = await call(plan.studioRequest);
  same(actualDryRun, plan.variantDryRun, 'Returned variant request differs from the validated dry-run');
  same(await call({ command: 'history', project: 'project' }), history, 'Translation dry-run changed history');
  const forged = structuredClone(plan.studioRequest);
  forged.dryRun = false;
  forged.edit.captions[rows[0].captionId] = 'Tampered';
  const rejected = await call(forged, false);
  if (!rejected.message.includes('provenance')) throw new Error('Forged variant text was accepted');
  const apply = { ...plan.studioRequest, dryRun: false };
  const outcome = await call(apply);
  if (outcome.previousRevision !== 4 || outcome.revision !== 5) throw new Error('Variant did not commit atomically');
  const editFile = jsonFile('studio-edit.json', apply.edit);
  same(cli(['studio', 'project', '--request', editFile, '--expected-revision', '4', '--key', planRequest.key]),
    outcome, 'Named CLI variant replay differs from MCP');
  same(cli(['request', jsonFile('apply-envelope.json', apply)]), outcome, 'JSON CLI variant replay differs from MCP');
  const after = await call({ command: 'status', project: 'project' });
  const originalSequence = project => project.sequences.find(sequence => sequence.id === 'captioned');
  same(originalSequence(after), originalSequence(before), 'Translation changed the source sequence');
  same(after.extensions['avw.transcripts.v1'], before.extensions['avw.transcripts.v1'], 'Translation changed the source transcript');
  const variant = after.sequences.find(sequence => sequence.id === 'spanish');
  if (!variant || variant.extensions['avw.variant'].language !== 'es'
      || variant.extensions['avw.variant'].sourceRevision !== 4
      || variant.extensions['avw.translationProvenance'].proposalSha256 !== plan.proposalSha256
      || variant.extensions['avw.translationProvenance'].proposal.provider !== 'synthetic-supplied-text') {
    throw new Error('Atomic variant lost its source or provider provenance');
  }
  const translated = await call({ command: 'render', project: 'project', sequence: 'spanish' });
  const srt = fs.readFileSync(path.join(path.dirname(translated.path), 'captions.srt'), 'utf8');
  const vtt = fs.readFileSync(path.join(path.dirname(translated.path), 'captions.vtt'), 'utf8');
  for (const text of ['Café listo', 'Hola mundo']) {
    if (!srt.includes(text) || !vtt.includes(text)) throw new Error('Translated sidecar is missing: ' + text);
  }
  if (srt.includes('Coffee demo') || vtt.includes('Coffee demo')) throw new Error('Translated variant retained source caption');
  const stale = await call({ ...planRequest, expectedRevision: 5 }, false);
  if (stale.code !== 'E_REVISION_CONFLICT') throw new Error('Stale translation proposal was accepted');
  if (sha(source) !== sourceSha || sha(importedFile) !== sourceSha
      || JSON.stringify(originalArtifacts.map(sha)) !== JSON.stringify(originalHashes)) {
    throw new Error('Original source or previous artifact changed');
  }
  const summary = { passed: true, officialMcpSdk: true, namedCliAndJsonEquivalent: true,
    generatedVideoAndTone: true, paginatedSourceCaptionContext: true, sourceCueProvenance: true,
    completeTranslationDryRun: true, forgedVariantRejected: true, atomicProviderProvenance: true,
    acceptanceAndReplay: true, staleProposalRejected: true, originalSequenceAndTranscriptUnchanged: true,
    originalSourceAndArtifactUnchanged: true, translatedSrtAndVttRendered: true,
    limitations: ['Caller-supplied text and synthetic tone; no translation, speech or script-shaping accuracy claim'] };
  jsonFile('summary.json', summary);
  console.log('Translation workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
