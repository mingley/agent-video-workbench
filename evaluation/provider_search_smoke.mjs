// Explicit local fixture-provider search via official MCP SDK and named CLI.
// The fixture returns prescribed observations; it is not a semantic model.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: provider_search_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const generated = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=8',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=8', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source], { encoding: 'utf8' });
if (generated.status !== 0) throw new Error(generated.stderr);
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sourceSha = sha(source);
const provider = path.join(root, 'fixture-provider');
const otherProvider = path.join(root, 'other-provider');
const marker = path.join(root, 'provider-ran');
fs.writeFileSync(provider, `#!/usr/bin/env python3
import argparse, json
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument('--avw-request'); p.add_argument('--avw-result'); args = p.parse_args()
r = json.loads(Path(args.avw_request).read_text())
assert r['task'] == 'moment-search-v1'
Path(__file__).with_name('provider-ran').write_text('fixture provider ran')
params = r['parameters']
result = {'schemaVersion': 1, 'kind': 'analysis', 'sourceSha256': r['source']['sha256'],
  'data': {'schemaVersion': 1, 'query': params['query'], 'candidates': [
    {'id': 'fixture-moment', 'startMs': 1000, 'endMs': 4000, 'title': 'Fixture moment',
     'reason': 'The fixture is programmed to return this interval', 'confidence': 0.83,
     'evidence': [{'kind': 'observation', 'modality': 'video', 'description': 'Generated test pattern'}]}
  ]}}
Path(args.avw_result).write_text(json.dumps(result))
`);
fs.chmodSync(provider, 0o755);
fs.copyFileSync(provider, otherProvider);
fs.chmodSync(otherProvider, 0o755);
const ledger = [];
const save = (name, value) => {
  const file = path.join(root, name);
  fs.writeFileSync(file, JSON.stringify(value, null, 2));
  return file;
};
const client = new Client({ name: 'avw-provider-search-fixture', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--analysis-provider', provider, 'mcp', '--root', root], stderr: 'inherit' });
const cli = (args, probe = ffprobe, configuredProvider = provider) => {
  const result = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', probe, '--analysis-provider', configuredProvider,
    '--workspace', root, ...args], { encoding: 'utf8', timeout: 300000 });
  if (result.status !== 0) throw new Error(result.stderr || result.stdout);
  return JSON.parse(result.stdout).result;
};
const rejectedCli = (requestFile, probe, configuredProvider) => {
  const result = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', probe, '--analysis-provider', configuredProvider,
    '--workspace', root, 'request', requestFile], { encoding: 'utf8', timeout: 300000 });
  const response = JSON.parse(result.stdout);
  if (result.status === 0 || response.ok || response.error.code !== 'E_INVALID_REQUEST'
      || !response.error.message.includes('generated search key')) {
    throw new Error('Copied provider request unexpectedly accepted: ' + result.stdout + result.stderr);
  }
};
try {
  await client.connect(transport);
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ request, response: value });
    save('commands.json', ledger);
    if (value.ok !== expectOk) throw new Error(JSON.stringify(value));
    return expectOk ? value.result : value.error;
  }
  await call({ command: 'create', project: 'project', name: 'Fixture provider search' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'import-source' });
  const search = { assetId: 'source', sourceSha256: sourceSha, query: 'Find the fixture moment',
    modality: 'visual', startMs: 0, endMs: 8000, maxResults: 2, targetDurationMs: 4000 };
  const request = { command: 'provider-search', project: 'project', expectedRevision: 1, search };
  const proposed = await call(request);
  const searchFile = save('search.json', search);
  if (JSON.stringify(cli(['provider-search', 'project', '--expected-revision', '1', '--request', searchFile])) !== JSON.stringify(proposed)
      || proposed.providerSha256 !== sha(provider) || !proposed.analysisRequest.key.startsWith('avw-search-v1-')
      || fs.existsSync(marker)) throw new Error('Preparing provider search was not explicit or reproducible');
  const queuedRequest = save('queued-request.json', proposed.analysisRequest);
  rejectedCli(queuedRequest, ffprobe, otherProvider);
  rejectedCli(queuedRequest, ffprobe + '.different-backend', provider);
  if (fs.existsSync(marker) || (await call({ command: 'jobs', project: 'project' })).length !== 0) {
    throw new Error('Wrong-config request ran a provider or queued work');
  }
  const job = await call(proposed.analysisRequest);
  if ((await call(proposed.analysisRequest)).id !== job.id) throw new Error('Search request did not replay');
  const worker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (worker.status !== 0) throw new Error(worker.stderr);
  const complete = await call({ command: 'job-status', project: 'project', id: job.id });
  if (complete.state !== 'succeeded' || !fs.existsSync(marker)) throw new Error(JSON.stringify(complete));
  const resultRequest = { command: 'provider-search-results', project: 'project', expectedRevision: 1,
    jobId: job.id, outputPrefix: 'highlight' };
  const result = await call(resultRequest);
  if (result.interpretation !== 'provider-supplied' || result.provider.programSha256 !== sha(provider)
      || result.provider.sourceSha256 !== sourceSha || result.items.length !== 1
      || result.items[0].providerCandidateId !== 'fixture-moment' || result.items[0].providerReportedConfidence !== 0.83
      || result.items[0].planningStatus.targetDurationDeltaMs !== -1000
      || result.items[0].evidence[0].verification !== 'provider-asserted' || !result.items[0].reviewRequired
      || result.items[0].planRequest.command !== 'plan-edit') {
    throw new Error('Provider result lost attribution or planning: ' + JSON.stringify(result));
  }
  if (JSON.stringify(cli(['provider-search-results', 'project', '--expected-revision', '1',
      '--job-id', job.id, '--output-prefix', 'highlight'])) !== JSON.stringify(result)) {
    throw new Error('CLI/MCP provider result differs');
  }
  const resultPath = complete.result.path;
  const resultBytes = fs.readFileSync(resultPath);
  try {
    fs.writeFileSync(resultPath, 'tampered');
    const error = await call(resultRequest, false);
    if (!error.message.includes('bytes changed')) throw new Error('Changed provider result was accepted');
  } finally { fs.writeFileSync(resultPath, resultBytes); }
  const previewJob = await call(result.items[0].previewRequest);
  const proxyWorker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (proxyWorker.status !== 0) throw new Error(proxyWorker.stderr);
  const proxy = await call({ command: 'artifact', project: 'project', id: previewJob.id });
  const preview = proxy.attachments.find(file => file.path.endsWith('proxy.mp4'));
  if (!proxy.verified || !preview || preview.sha256 !== sha(preview.path)) throw new Error('Provider source preview was not verified');
  const plan = await call(result.items[0].planRequest);
  if (!plan.readyToApply || !plan.planSha256) throw new Error('Provider candidate did not produce a reviewable plan');
  await call({ command: 'apply-edit-plan', project: 'project', expectedRevision: 1,
    plan: result.items[0].planRequest.plan, planSha256: plan.planSha256, key: 'apply-provider-candidate' });
  const render = await call({ command: 'render-start', project: 'project', sequence: 'highlight-1',
    expectedRevision: 2, key: 'render-provider-candidate', noLaunch: true });
  const renderWorker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (renderWorker.status !== 0) throw new Error(renderWorker.stderr);
  const master = await call({ command: 'artifact', project: 'project', id: render.id });
  if (!master.verified || master.sha256 !== sha(master.path) || sha(source) !== sourceSha) {
    throw new Error('Provider candidate artifact or original source integrity failed');
  }
  const stale = await call(resultRequest, false);
  if (stale.code !== 'E_REVISION_CONFLICT') throw new Error('Stale provider result was accepted');
  save('summary.json', { passed: true, officialMcpSdk: true, namedCliEquivalent: true,
    providerNotRunUntilExplicitQueue: true, copiedProviderAndBackendRequestsRejected: true,
    providerResultHashAndAttributionVerified: true, tamperedResultRejected: true,
    proxyAndPlannedRenderVerified: true, staleRevisionRejected: true, sourceUnchanged: true,
    limitation: 'Fixture provider returns a prescribed result; no semantic-model quality is established.' });
  console.log('Provider search workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
