// Source evidence and caller-selected highlights through official MCP and CLI.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: discovery_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const generate = spawnSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=8',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=8', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source], { encoding: 'utf8' });
if (generate.status !== 0) throw new Error(generate.stderr);
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const originalSha = sha(source);
const ledger = [];
const write = (name, value) => {
  const file = path.join(root, name);
  fs.writeFileSync(file, JSON.stringify(value, null, 2));
  return file;
};
const client = new Client({ name: 'avw-source-discovery', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
const command = (args, input = undefined) => {
  const result = spawnSync(avw, args, { input, encoding: 'utf8', timeout: 300000 });
  if (result.status !== 0) throw new Error(result.stderr || result.stdout);
  return result.stdout;
};
const cli = args => JSON.parse(command(['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root, ...args])).result;
try {
  await client.connect(transport);
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ request, response: value });
    write('commands.json', ledger);
    if (value.ok !== expectOk) throw new Error(JSON.stringify(value));
    return expectOk ? value.result : value.error;
  }
  await call({ command: 'create', project: 'project', name: 'Generated discovery fixture' });
  await call({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'import-source' });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 1, key: 'fixture-transcript',
    transcript: { assetId: 'source', language: 'en', provider: 'generated-test-not-speech', cues: [
      { id: 'opening', startMs: 500, endMs: 1500, text: 'Opening explanation' },
      { id: 'answer', startMs: 2100, endMs: 2900, text: 'A useful answer' },
      { id: 'closing', startMs: 5000, endMs: 6000, text: 'Closing thought' },
    ] } });
  const query = { assetId: 'source', windowMs: 3000, limit: 2 };
  const request = { command: 'source-evidence', project: 'project', expectedRevision: 2, query };
  const page = await call(request);
  if (page.sourceSha256 !== originalSha || page.revision !== 2 || page.totalWindows !== 3 || page.nextOffset !== 2
      || page.items[0].cueExcerpts[0].id !== 'opening' || !page.items[0].previewRequest
      || page.transcriptStatus !== 'available') throw new Error('Discovery evidence was incomplete: ' + JSON.stringify(page));
  const queryFile = write('query.json', query);
  if (JSON.stringify(cli(['source-evidence', 'project', '--expected-revision', '2', '--request', queryFile])) !== JSON.stringify(page)) {
    throw new Error('CLI/MCP source evidence differs');
  }
  const later = await call({ ...request, query: { ...query, offset: page.nextOffset } });
  if (later.items.length !== 1 || later.items[0].sourceStartMs !== 6000 || later.nextOffset !== null) {
    throw new Error('Discovery evidence pagination was incorrect');
  }
  const previewJob = await call(page.items[0].previewRequest);
  const replay = await call(page.items[0].previewRequest);
  if (previewJob.id !== replay.id) throw new Error('Preview request is not idempotent');
  command(['worker', path.join(root, 'project')]);
  const proxy = await call({ command: 'artifact', project: 'project', id: previewJob.id });
  const video = proxy.attachments.find(file => file.path.endsWith('proxy.mp4'));
  if (!proxy.verified || !video || video.sha256 !== sha(video.path)) throw new Error('Proxy was not verified');
  const probe = spawnSync(ffprobe, ['-v', 'error', '-show_streams', '-of', 'json', video.path], { encoding: 'utf8' });
  if (probe.status !== 0 || !JSON.parse(probe.stdout).streams.some(stream => stream.codec_type === 'audio')) {
    throw new Error('Source context proxy lacks audio');
  }
  const brief = { objective: 'Retain a complete introductory explanation', targetDurationMs: 4000, candidates: [
    { id: 'first-cut', outputId: 'short-a', assetId: 'source', sourceSha256: originalSha,
      startMs: 0, endMs: 4000, title: 'Introductory moment', reason: 'The supplied cue identifies the opening',
      evidence: [{ kind: 'transcript-cue', transcriptSha256: page.transcript.sha256, cueId: 'opening' },
        { kind: 'observation', modality: 'video', description: 'Caller observed generated test pattern' }] },
  ] };
  const proposal = await call({ command: 'prepare-highlights', project: 'project', expectedRevision: 2, brief });
  if (proposal.interpretation !== 'caller-supplied' || proposal.items[0].sourceSha256 !== originalSha
      || proposal.items[0].evidence[0].cue.id !== 'opening' || proposal.items[0].evidence[1].verification !== 'caller-asserted'
      || !proposal.items[0].planRequest || !proposal.items[0].reviewRequired) {
    throw new Error('Proposal lost provenance or planning: ' + JSON.stringify(proposal));
  }
  const briefFile = write('brief.json', brief);
  if (JSON.stringify(cli(['prepare-highlights', 'project', '--expected-revision', '2', '--request', briefFile])) !== JSON.stringify(proposal)) {
    throw new Error('CLI/MCP highlight preparation differs');
  }
  const plan = await call(proposal.items[0].planRequest);
  if (!plan.readyToApply || !plan.planSha256 || plan.cuts[0].assetId !== 'source') {
    throw new Error('The grounded draft did not yield a ready plan: ' + JSON.stringify(plan));
  }
  const apply = { command: 'apply-edit-plan', project: 'project', expectedRevision: 2,
    plan: proposal.items[0].planRequest.plan, planSha256: plan.planSha256, key: 'apply-reviewed-highlight' };
  await call({ ...apply, dryRun: true });
  await call(apply);
  const render = await call({ command: 'render-start', project: 'project', sequence: 'short-a',
    expectedRevision: 3, key: 'render-highlight', noLaunch: true });
  command(['worker', path.join(root, 'project')]);
  const job = await call({ command: 'job-status', project: 'project', id: render.id });
  if (job.state !== 'succeeded') throw new Error(JSON.stringify(job));
  const master = await call({ command: 'artifact', project: 'project', id: render.id });
  if (!master.verified || master.revision !== 3 || master.sha256 !== sha(master.path)) {
    throw new Error('Reviewed candidate did not produce a verified artifact');
  }
  const stale = await call(request, false);
  if (stale.code !== 'E_REVISION_CONFLICT' || sha(source) !== originalSha) {
    throw new Error('Stale page was accepted or the source changed');
  }
  write('summary.json', { passed: true, officialMcpSdk: true, namedCliEquivalent: true,
    transcriptCueAndSourceIdentityBound: true, sourceProxyVerifiedWithAudio: true,
    callerObservationClearlyAttributed: true, planAppliedAndRendered: true,
    staleRevisionRejected: true, originalUnchanged: true });
  console.log('Source discovery workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
