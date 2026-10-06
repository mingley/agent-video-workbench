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
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const value = response.structuredContent ?? JSON.parse(response.content[0].text);
    ledger.push({ request, response: value });
    fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2));
    if (value.ok !== expectOk) throw new Error(JSON.stringify(value));
    return expectOk ? value.result : value.error;
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
  // Paper-edit planning must catch clipped cues before any project mutation.
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 3,
    key: 'reviewed-timing-cues', transcript: { assetId: 'phone', language: 'en',
      provider: 'reviewed-generated-burst-fixture', cues: [
        { id: 'first-burst', startMs: 800, endMs: 1200, text: 'First synchronized light and audio burst' },
        { id: 'second-burst', startMs: 2050, endMs: 2300, text: 'Second synchronized light and audio burst' },
      ] } });
  const plan = { brief: { objective: 'Retain both complete bursts and their natural source timing' },
    edit: { ...request.edit, outputId: 'planned', name: 'Reviewed HDR paper edit <test>' },
    decisions: [
      { cutId: 'first', reason: 'Keep the complete first burst and surrounding context' },
      { cutId: 'second', reason: 'Keep the complete second burst <no overlay>' },
    ] };
  const badPlan = structuredClone(plan); badPlan.edit.cuts[0].startMs = 900;
  const blocked = await call({ command: 'plan-edit', project: 'project', expectedRevision: 4, plan: badPlan });
  if (blocked.readyToApply || !blocked.cuts[0].findings.some(f => f.risk === 'speech-boundary')) {
    throw new Error('Clipped source cue did not block planning');
  }
  const rejected = await call({ command: 'apply-edit-plan', project: 'project', expectedRevision: 4,
    key: 'reject-clipped-plan', plan: badPlan, planSha256: blocked.planSha256 }, false);
  if (rejected.code !== 'E_INVALID_REQUEST' || (await call({ command: 'status', project: 'project' })).revision !== 4) {
    throw new Error('Blocked plan changed history');
  }
  // This fixture uses generated sync tones, not actual reviewed human speech.
  plan.decisions[1].reviews = [{ risk: 'speech-handle',
    note: 'Checked the complete generated 2.25–2.35s burst; the selected 2–2.5s source interval retains it and 150ms trailing silence' }];
  const planned = await call({ command: 'plan-edit', project: 'project', expectedRevision: 4, plan });
  if (!planned.readyToApply || planned.cuts.length !== 2 || planned.omittedSourceRanges.length !== 3) {
    throw new Error('Reviewed plan or omitted source union is wrong');
  }
  const planFile = path.join(root, 'plan-envelope.json');
  fs.writeFileSync(planFile, JSON.stringify({ command: 'plan-edit', project: 'project', expectedRevision: 4, plan }));
  const cliPlan = spawnSync(avw, ['--workspace', root, 'request', planFile], { encoding: 'utf8' });
  if (cliPlan.status !== 0 || JSON.stringify(JSON.parse(cliPlan.stdout).result) !== JSON.stringify(planned)) {
    throw new Error('CLI/MCP planning differs');
  }
  const windows = planned.cuts.flatMap(cut => cut.contextWindows);
  const contextJobs = [];
  for (const window of windows) contextJobs.push(await call(window.previewRequest));
  const contextWorker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (contextWorker.status !== 0) throw new Error(contextWorker.stderr);
  let contextAudioBurstSeconds;
  for (const [index, contextJob] of contextJobs.entries()) {
    const evidence = await call({ command: 'artifact', project: 'project', id: contextJob.id });
    const attachment = evidence.attachments.find(file => file.path.endsWith('proxy.mp4'));
    if (!evidence.verified || !attachment || attachment.sha256 !== sha(attachment.path)) {
      throw new Error('Source context attachment was not hash verified');
    }
    const probe = spawnSync(ffprobe, ['-v', 'error', '-count_frames', '-show_streams', '-of', 'json', attachment.path], { encoding: 'utf8' });
    if (probe.status !== 0) throw new Error(probe.stderr);
    const streams = JSON.parse(probe.stdout).streams;
    const frames = Number(streams.find(s => s.codec_type === 'video').nb_read_frames);
    const expectedFrames = (windows[index].sourceEndMs - windows[index].sourceStartMs) * 30 / 1000;
    if (Math.abs(frames - expectedFrames) > 1 || !streams.some(s => s.codec_type === 'audio')) {
      throw new Error('Source-context duration or audio was lost');
    }
    if (index === 0) {
      const pcm = spawnSync(ffmpeg, ['-v', 'error', '-i', attachment.path, '-vn', '-ac', '1', '-ar', '48000', '-f', 'f32le', '-'], { maxBuffer: 8 * 1024 * 1024 });
      if (pcm.status !== 0) throw new Error(pcm.stderr.toString());
      let first = -1;
      for (let sample = 0; sample < pcm.stdout.length / 4; sample++) {
        if (Math.abs(pcm.stdout.readFloatLE(sample * 4)) > 0.1) { first = sample; break; }
      }
      contextAudioBurstSeconds = first / 48000;
      if (first < 0 || Math.abs(contextAudioBurstSeconds - 1) > 0.025) throw new Error('Context audio source offset changed');
    }
  }
  if ((await call({ command: 'status', project: 'project' })).revision !== 4) throw new Error('Context review changed history');
  const applyPlan = { command: 'apply-edit-plan', project: 'project', expectedRevision: 4,
    key: 'apply-reviewed-plan', plan, planSha256: planned.planSha256 };
  await call({ ...applyPlan, dryRun: true });
  if ((await call({ command: 'status', project: 'project' })).revision !== 4) throw new Error('Plan dry-run changed history');
  const applied = await call(applyPlan);
  fs.writeFileSync(planFile, JSON.stringify(applyPlan));
  const cliApplied = spawnSync(avw, ['--workspace', root, 'request', planFile], { encoding: 'utf8' });
  if (cliApplied.status !== 0 || JSON.stringify(JSON.parse(cliApplied.stdout).result) !== JSON.stringify(applied)) {
    throw new Error('CLI/MCP plan replay differs');
  }
  const plannedJob = await call({ command: 'render-start', project: 'project', sequence: 'planned',
    expectedRevision: 5, key: 'render-reviewed-plan', noLaunch: true });
  await call({ command: 'transcript-import', project: 'project', expectedRevision: 5,
    key: 'later-cue-correction', transcript: { assetId: 'phone', language: 'en', provider: 'correction',
      cues: [{ id: 'first-burst', startMs: 800, endMs: 1250, text: 'Corrected timing evidence' }] } });
  const stale = await call({ command: 'edit-preflight', project: 'project', sequence: 'planned' });
  if (stale.editPlan.status !== 'stale') throw new Error('Changed transcript retained false current review');
  const plannedWorker = spawnSync(avw, ['worker', path.join(root, 'project')], { encoding: 'utf8', timeout: 300000 });
  if (plannedWorker.status !== 0) throw new Error(plannedWorker.stderr);
  const plannedStatus = await call({ command: 'job-status', project: 'project', id: plannedJob.id });
  if (plannedStatus.state !== 'succeeded') throw new Error(JSON.stringify(plannedStatus));
  const frozenPlan = JSON.parse(fs.readFileSync(plannedStatus.result.manifest));
  if (frozenPlan.revision !== 5 || frozenPlan.editPreflight.editPlan.status !== 'current'
    || frozenPlan.editPreflight.editPlan.planSha256 !== planned.planSha256
    || frozenPlan.verification.expectedFrames !== 90) throw new Error('Frozen planned edit lost review or media contract');
  await call({ command: 'delivery', project: 'project', id: plannedJob.id, destination: 'planned-review' });
  await call({ command: 'backup', project: 'project', destination: 'planned-backup' });
  const restoredPlan = await call({ command: 'edit-preflight', project: 'planned-backup', sequence: 'planned' });
  if (restoredPlan.editPlan.status !== 'stale' || restoredPlan.editPlan.decisions[1].reviews.length !== 1) {
    throw new Error('Backup lost decisions or stale review state');
  }
  fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify({ passed: true, officialMcpSdk: true,
    cliMcpReplayEquivalent: true, frozenHdrRevision: 2, currentRevision: 6, sourceUnchanged: true,
    editorialPlanning: { clippedCueRefused: true, cliMcpPlanAndReplayEquivalent: true,
      sourceContextPreviews: windows.length, contextAudioBurstSeconds, frozenPlannedRevision: 5,
      transcriptChangeInvalidatesReview: true, backupRetainsDecisions: true },
    verifiedHdrMaster: artifact.path, verifiedSdrPreview: preview, backupRetainsPolicy: true }, null, 2));
  console.log('Source-preserving MCP workflow passed:', path.join(root, 'summary.json'));
} finally {
  await client.close();
}
