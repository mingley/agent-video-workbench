// Exercise source-preserving edits through the official MCP SDK and durable jobs.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, fixture, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: preserve_agent_smoke.mjs AVW FFMPEG FFPROBE PQ_FIXTURE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
fs.copyFileSync(fixture, path.join(root, 'original.mp4'));
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const originalSha = sha(path.join(root, 'original.mp4'));
const client = new Client({ name: 'avw-natural-edit-conformance', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
const ledger = [];
try {
  await client.connect(transport);
  const tools = await client.listTools();
  if (!JSON.stringify(tools).includes('edit-preflight') || !JSON.stringify(tools).includes('assemble')) {
    throw new Error('New workflow is absent from the MCP schema');
  }
  async function call(request) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ request, response: value });
    fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2));
    if (!value.ok) throw new Error(JSON.stringify(value));
    return value.result;
  }
  const doctor = await call({ command: 'doctor' });
  if (!doctor.sourcePreservingHdr.ready) throw new Error('HDR encoder is unavailable');
  await call({ command: 'create', project: 'project', name: 'Natural HDR agent edit' });
  await call({ command: 'import', project: 'project', source: 'original.mp4', id: 'phone',
    expectedRevision: 0, key: 'import-original' });
  const request = { command: 'assemble', project: 'project', expectedRevision: 1, key: 'natural-baseline',
    edit: { outputId: 'natural', name: 'Source-matched cuts', quality: 'lossless', cuts: [
      { id: 'first', assetId: 'phone', startMs: 500, endMs: 1500 },
      { id: 'second', assetId: 'phone', startMs: 2000, endMs: 2500 },
    ] } };
  await call({ ...request, dryRun: true });
  if ((await call({ command: 'status', project: 'project' })).revision !== 1) throw new Error('Dry-run changed history');
  const outcome = await call(request);
  const requestFile = path.join(root, 'retry.json');
  fs.writeFileSync(requestFile, JSON.stringify(request));
  const retry = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root,
    'request', requestFile], { encoding: 'utf8' });
  if (retry.status !== 0 || JSON.stringify(JSON.parse(retry.stdout).result) !== JSON.stringify(outcome)) {
    throw new Error('CLI/MCP replay differs');
  }
  const preflight = await call({ command: 'edit-preflight', project: 'project', sequence: 'natural' });
  if (preflight.outputColor.transfer !== 'smpte2084' || preflight.automaticCaptions || preflight.automaticCrop) {
    throw new Error('Source intent was changed');
  }
  const job = await call({ command: 'render-start', project: 'project', sequence: 'natural',
    expectedRevision: 2, key: 'render-natural', noLaunch: true });
  const state = await call({ command: 'status', project: 'project' });
  await call({ command: 'apply', project: 'project', batch: { schemaVersion: '1.0.0', projectId: state.projectId,
    baseRevision: 2, idempotencyKey: 'later-agent-edit', operations: [
      { id: 'rename', op: 'project.rename', params: { name: 'Later edit' } },
    ] } });
  const worker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (worker.status !== 0) throw new Error(worker.stderr);
  const status = await call({ command: 'job-status', project: 'project', id: job.id });
  if (status.state !== 'succeeded') throw new Error(JSON.stringify(status));
  const artifact = await call({ command: 'artifact', project: 'project', id: job.id });
  if (!artifact.verified || artifact.revision !== 2 || artifact.sha256 !== sha(artifact.path)) {
    throw new Error('Frozen artifact was not verified');
  }
  const manifest = JSON.parse(fs.readFileSync(status.result.manifest));
  if (manifest.outputColor.pixelFormat !== 'yuv420p10le' || manifest.verification.expectedFrames !== 90
    || manifest.snapshot.revision !== 2 || manifest.delivery.reviewVideo !== 'review-sdr.mp4') {
    throw new Error('HDR contract or frozen snapshot was lost');
  }
  const reviewArtifact = await call({ command: 'artifact', project: 'project', id: job.id, preview: true });
  if (!reviewArtifact.verified || reviewArtifact.role !== 'sdr-review-preview'
    || reviewArtifact.sha256 !== sha(reviewArtifact.path)) throw new Error('Review artifact is not verified');
  await call({ command: 'delivery', project: 'project', id: job.id, destination: 'review' });
  const preview = path.join(root, 'review', job.id, 'review-sdr.mp4');
  const entries = manifest.delivery.files.filter(file => file.path === 'review-sdr.mp4');
  if (entries.length !== 1 || entries[0].sha256 !== sha(preview)) throw new Error('Preview hash mismatch');
  if (originalSha !== sha(path.join(root, 'original.mp4'))) throw new Error('Original was rewritten');
  await call({ command: 'backup', project: 'project', destination: 'backup' });
  const backup = await call({ command: 'edit-preflight', project: 'backup', sequence: 'natural' });
  if (backup.outputColor.transfer !== 'smpte2084' || backup.revision !== 3) throw new Error('Backup lost delivery intent');
  fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify({ passed: true, officialMcpSdk: true,
    cliMcpReplayEquivalent: true, frozenHdrRevision: 2, currentRevision: 3, sourceUnchanged: true,
    verifiedHdrMaster: artifact.path, verifiedSdrPreview: preview, backupRetainsPolicy: true }, null, 2));
  console.log('Source-preserving MCP workflow passed:', path.join(root, 'summary.json'));
} finally {
  await client.close();
}
