// Exercise independent delivery briefs against a real frozen multi-format batch.
import { createHash, randomUUID } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [avw, ffmpeg, ffprobe, font, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: delivery_batch_smoke.mjs AVW FFMPEG FFPROBE FONT NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
fs.copyFileSync(font, path.join(root, 'font.ttf'));
const source = path.join(root, 'source.mp4');
const generated = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=160x90:r=30:d=3',
  '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
  '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv', source],
  { encoding: 'utf8', timeout: 120000 });
if (generated.status !== 0) throw new Error(generated.stderr);
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sourceHash = sha(source);
const ledger = [];
const client = new Client({ name: 'avw-batch-delivery', version: '1.0.0' });
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
function assertNoSecrets(dir, secrets) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) assertNoSecrets(file, secrets);
    else {
      const bytes = fs.readFileSync(file);
      for (const secret of secrets) {
        if (bytes.includes(Buffer.from(secret))) throw new Error(`Private fixture leaked in ${file}`);
      }
    }
  }
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
  await call({ command: 'create', project: 'project', name: 'Generated batch delivery' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'batch-import-source' });
  await call({ command: 'import', project: 'project', source: 'font.ttf', id: 'font',
    expectedRevision: 1, key: 'batch-import-font' });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 2,
    key: 'batch-reviewed-transcript', transcript: { assetId: 'source', language: 'en', provider: 'generated-fixture',
      cues: [{ id: 'cue', startMs: 300, endMs: 1100, text: 'Hello' }] } });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'other',
    expectedRevision: 3, key: 'batch-import-unrelated' });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 4,
    key: 'batch-unrelated-transcript', transcript: { assetId: 'other', language: 'en', provider: 'generated-fixture',
      cues: [{ id: 'private', startMs: 300, endMs: 1100, text: 'SECRET_UNRELATED_TRANSCRIPT_cb8d' }] } });
  const project = await call({ command: 'status', project: 'project' });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: project.projectId,
    baseRevision: 5, idempotencyKey: 'batch-private-sequence', operations: [
      { id: 'private-sequence', op: 'sequence.add', params: { id: 'private_sequence', name: 'SECRET_OTHER_SEQUENCE_420e',
        width: 180, height: 180, frameRate: { numerator: 30, denominator: 1 } } },
    ] } });
  for (const [id, width, height, revision] of [['square', 180, 180, 6], ['vertical', 90, 160, 7]]) {
    await call({ command: 'compose', project: 'project', expectedRevision: revision, key: `compose-${id}`,
      edit: { outputId: id, name: `SECRET_INTERNAL_TITLE_${id}`, width, height, fontAssetId: 'font', fontSize: 12,
        cuts: [{ id: 'clip', assetId: 'source', startMs: 0, endMs: 2000 }] } });
  }
  const batch = await call({ command: 'batch-start', project: 'project', sequences: ['square', 'vertical'],
    expectedRevision: 8, key: 'freeze-batch', noLaunch: true });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: project.projectId,
    baseRevision: 8, idempotencyKey: 'rename-after-batch', operations: [
      { id: 'rename', op: 'project.rename', params: { name: 'Later project' } },
    ] } });
  const worker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (worker.status !== 0) throw new Error(worker.stderr);
  const state = await call({ command: 'batch-status', project: 'project', id: batch.id });
  if (!state.complete || state.items.length !== 2 || !state.items.every(item => item.state === 'succeeded')) {
    throw new Error('Batch did not render: ' + JSON.stringify(state));
  }
  const renderManifestPath = state.items[0].result.manifest;
  const renderManifest = JSON.parse(fs.readFileSync(renderManifestPath));
  if (!JSON.stringify(renderManifest.snapshot).includes('SECRET_UNRELATED_TRANSCRIPT_cb8d')
      || !JSON.stringify(renderManifest.snapshot).includes('SECRET_OTHER_SEQUENCE_420e')) {
    throw new Error('Private fixture did not enter the frozen project snapshot');
  }
  const extra = path.join(path.dirname(renderManifestPath), 'private-note.txt');
  fs.writeFileSync(extra, 'SECRET_EXTRA_FILE_147e');
  // Preserve floating-point values in the frozen snapshot; JSON.stringify would
  // turn integral-valued floats into integers and invalidate snapshot identity.
  const inject = spawnSync('python3', ['-c', `import json, sys
p = sys.argv[1]
with open(p) as f: manifest = json.load(f)
manifest['editPreflight'] = {'editPlan': {'brief': {'objective': 'SECRET_SELECTED_OBJECTIVE_906d'}, 'decisions': [{'reason': 'SECRET_SELECTED_CUT_NOTE_582b'}]}}
manifest['delivery']['files'].append({'path': 'private-note.txt', 'sha256': sys.argv[2]})
with open(p, 'w') as f: json.dump(manifest, f, ensure_ascii=False)
`, renderManifestPath, sha(extra)], { encoding: 'utf8' });
  if (inject.status !== 0) throw new Error(inject.stderr);
  await call({ command: 'studio', project: 'project', expectedRevision: 9, key: 'private-review-record',
    edit: { action: 'review-add', id: 'private-review', jobId: state.items[0].id, atMs: 100, endMs: 200,
      actor: 'Fixture', text: 'SECRET_STORED_REVIEW_7d16' } });
  const brief = { schemaVersion: 1, outputs: [
    { sequenceId: 'square', brief: { schemaVersion: 1, dimensions: { width: 180, height: 180 },
      maxDurationMs: 2100, maxBytes: 100000000, requiredSidecars: ['srt', 'vtt'] },
      metadata: { title: 'Square version', tags: ['demo'] } },
    { sequenceId: 'vertical', brief: { schemaVersion: 1, dimensions: { width: 90, height: 160 }, maxBytes: 1 },
      metadata: { title: 'Vertical version', description: 'A caller-authored draft description.' } },
  ] };
  const request = { command: 'batch-delivery-check', project: 'project', id: batch.id, brief };
  const report = await call(request);
  if (report.status !== 'fail' || report.frozenRevision !== 8 || report.items.length !== 2
      || report.items[0].sequenceId !== 'square' || report.items[0].status !== 'pass'
      || report.items[1].sequenceId !== 'vertical' || report.items[1].status !== 'fail'
      || report.items[0].metadataSource !== 'caller-supplied' || report.items[0].metadata.title !== 'Square version'
      || report.items[0].report.artifact.revision !== 8
      || sha(report.items[0].report.artifact.path) !== report.items[0].report.artifact.sha256
      || report.items[0].report.checks.filter(check => check.constraint === 'requiredSidecar'
        && check.status === 'pass' && check.actual.cueCount === 1).length !== 2
      || !report.items[1].report.checks.some(check => check.constraint === 'maxBytes' && check.status === 'fail')) {
    throw new Error('Independent delivery checks are wrong: ' + JSON.stringify(report));
  }
  const requestFile = writeJson('request.json', request);
  const briefFile = writeJson('brief.json', brief);
  for (const args of [['request', requestFile], ['batch-delivery-check', 'project', batch.id, '--request', briefFile]]) {
    if (JSON.stringify(cli(args)) !== JSON.stringify(report)) throw new Error('CLI/MCP batch delivery differs');
  }
  const packageRequest = { command: 'batch-delivery', project: 'project', id: batch.id,
    destination: 'package-mcp', brief };
  const result = await call(packageRequest);
  const packaged = result.manifest;
  if (packaged.package.state !== 'complete' || packaged.package.packagedOutputs !== 2
      || packaged.status !== 'fail' || packaged.items[0].status !== 'pass' || packaged.items[1].status !== 'fail'
      || JSON.stringify(JSON.parse(fs.readFileSync(path.join(result.path, 'manifest.json')))) !== JSON.stringify(packaged)
      || fs.existsSync(path.join(result.path, 'delivery.incomplete'))) {
    throw new Error('Batch package manifest is wrong: ' + JSON.stringify(packaged));
  }
  const privateSentinels = ['SECRET_UNRELATED_TRANSCRIPT_cb8d', 'SECRET_OTHER_SEQUENCE_420e',
    'SECRET_INTERNAL_TITLE_square', 'SECRET_INTERNAL_TITLE_vertical', 'SECRET_SELECTED_OBJECTIVE_906d',
    'SECRET_SELECTED_CUT_NOTE_582b', 'SECRET_EXTRA_FILE_147e', 'SECRET_STORED_REVIEW_7d16'];
  assertNoSecrets(result.path, privateSentinels);
  for (const [index, item] of packaged.items.entries()) {
    const master = path.join(result.path, item.package.masterPath);
    if (item.package.status !== 'packaged' || sha(master) !== item.report.artifact.sha256
        || 'path' in item.report.artifact || 'manifest' in item.report.artifact
        || !fs.existsSync(path.join(result.path, item.package.reviewPath))) {
      throw new Error('Packaged output identity or hash is wrong: ' + JSON.stringify(item));
    }
    if (index === 0) {
      for (const check of item.report.checks.filter(check => check.constraint === 'requiredSidecar')) {
        const file = path.join(path.dirname(master), 'captions.' + check.requested);
        if (check.status !== 'pass' || sha(file) !== check.actual.sha256) {
          throw new Error('Packaged caption sidecar differs from its checked hash');
        }
      }
    }
  }
  const cliPackage = cli(['batch-delivery', 'project', batch.id, 'package-cli', '--request', briefFile]);
  const jsonPackageRequest = writeJson('package-request.json', { ...packageRequest, destination: 'package-json' });
  const jsonPackage = cli(['request', jsonPackageRequest]);
  if (JSON.stringify(cliPackage.manifest) !== JSON.stringify(packaged)
      || JSON.stringify(jsonPackage.manifest) !== JSON.stringify(packaged)) {
    throw new Error('CLI/MCP batch package reports differ');
  }
  const packageManifestHash = sha(path.join(result.path, 'manifest.json'));
  const duplicate = await call(packageRequest, false);
  if (duplicate.code !== 'E_IO' || sha(path.join(result.path, 'manifest.json')) !== packageManifestHash) {
    throw new Error('Existing batch package was overwritten or not rejected');
  }
  const missingBrief = await call({ ...request, brief: { ...brief, outputs: [brief.outputs[0]] } });
  if (missingBrief.status !== 'unknown' || missingBrief.items[0].status !== 'pass'
      || missingBrief.items[1].status !== 'unknown') throw new Error('Missing sibling brief was not reported');
  const master = report.items[0].report.artifact.path;
  const bytes = fs.readFileSync(master);
  try {
    fs.writeFileSync(master, 'tampered');
    const altered = await call(request);
    if (altered.items[0].status !== 'unknown' || altered.items[1].status !== 'fail'
        || !altered.items[0].verificationError.message.includes('bytes changed')) {
      throw new Error('Tampered sibling was not isolated: ' + JSON.stringify(altered));
    }
    const partial = await call({ ...packageRequest, destination: 'package-partial' });
    if (partial.manifest.package.state !== 'partial' || partial.manifest.package.packagedOutputs !== 1
        || partial.manifest.items[0].package.status !== 'unavailable'
        || partial.manifest.items[1].package.status !== 'packaged'
        || !fs.existsSync(path.join(partial.path, partial.manifest.items[1].package.masterPath))) {
      throw new Error('Package did not retain the healthy sibling: ' + JSON.stringify(partial));
    }
  } finally { fs.writeFileSync(master, bytes); }
  const reviewPage = fs.readFileSync(path.join(result.path, packaged.items[0].package.reviewPath), 'utf8');
  const artifactSha = report.items[0].report.artifact.sha256;
  if (!reviewPage.includes(`data-review-project='${project.projectId}'`)
      || !reviewPage.includes(`data-review-sha256='${artifactSha}'`)
      || !reviewPage.includes('data-feedback-draft') || !reviewPage.includes('Square version')) {
    throw new Error('Share-safe review page lost feedback identity or caller title');
  }
  const draft = { schemaVersion: 1, draftId: randomUUID(), projectId: project.projectId,
    jobId: state.items[0].id, artifactRevision: 8, artifactSha256: artifactSha,
    items: [{ id: 'note-1', atMs: 300, endMs: 301, actor: 'Reviewer', text: 'Public review feedback' }] };
  const imported = await call({ command: 'review-import', project: 'project', expectedRevision: 10,
    key: 'share-review-import', draft });
  if (imported.outcome.revision !== 11) throw new Error('Feedback from share-safe package did not import');
  if ((await call({ command: 'status', project: 'project' })).revision !== 11 || sha(source) !== sourceHash) {
    throw new Error('Batch report changed project history or original bytes');
  }
  writeJson('summary.json', { passed: true, officialMcpSdk: true, namedCliAndJsonEquivalent: true,
    frozenRevision: report.frozenRevision, currentRevision: 11, squareAndCaptionSidecarsPassed: true, verticalFailedSize: true,
    missingBriefUnknown: true, tamperedSiblingIsolated: true, callerMetadataPreserved: true,
    packagesMatchChecks: true, copiedCaptionHashesMatch: true, packageCliAndMcpEquivalent: true,
    privateProjectDataExcludedRecursively: true, exportedReviewIdentityImported: true,
    existingPackageNotOverwritten: true, sourceUnchanged: true });
  console.log('Frozen batch delivery passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
