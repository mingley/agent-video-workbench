// Exercise an actual browser-exported draft on a disposable generated project.
// This test temporarily alters the generated render to test tamper rejection;
// do not run it against a creator's project or original media.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const [avw, projectPath, draftFile, sdkRoot, output] = process.argv.slice(2);
if (!output) throw new Error('Usage: review_import_smoke.mjs AVW GENERATED_PROJECT BROWSER_DRAFT SDK_ROOT NEW_OUTPUT');
const project = fs.realpathSync(projectPath);
const root = path.dirname(project);
const locator = path.basename(project);
const draft = JSON.parse(fs.readFileSync(draftFile));
fs.mkdirSync(output, { recursive: false });
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
const client = new Client({ name: 'avw-browser-feedback-import', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw, args: ['mcp', '--root', root], stderr: 'inherit' });
const ledger = [];
const record = entry => {
  ledger.push(entry);
  fs.writeFileSync(path.join(output, 'commands.json'), JSON.stringify(ledger, null, 2) + '\n');
};
const cli = args => {
  const run = spawnSync(avw, ['--workspace', root, ...args], { encoding: 'utf8', timeout: 30000 });
  const result = JSON.parse(run.stdout);
  record({ args, result });
  if (run.status !== 0 || !result.ok) throw new Error(JSON.stringify(result));
  return result.result;
};
try {
  await client.connect(transport);
  async function call(request, expectOk = true) {
    const response = await client.callTool({ name: 'avw', arguments: request });
    const result = response.structuredContent ?? JSON.parse(response.content[0].text);
    record({ request, result });
    if (result.ok !== expectOk) throw new Error(JSON.stringify(result));
    return expectOk ? result.result : result.error;
  }
  const before = await call({ command: 'status', project: locator });
  const revision = before.revision;
  const key = 'browser-feedback-import-v1';
  const request = { command: 'review-import', project: locator, expectedRevision: revision, key, draft };
  const preview = cli(['review-import', locator, '--request', draftFile,
    '--expected-revision', String(revision), '--key', key, '--dry-run']);
  if (JSON.stringify(await call({ command: 'status', project: locator })) !== JSON.stringify(before)) {
    throw new Error('Preview changed the project');
  }
  const imported = await call(request);
  if (JSON.stringify(imported) !== JSON.stringify(preview) || imported.outcome.revision !== revision + 1
      || imported.items.length !== draft.items.length) throw new Error('MCP import and named CLI preview differ');
  const retry = cli(['review-import', locator, '--request', draftFile,
    '--expected-revision', String(revision), '--key', key]);
  if (JSON.stringify(retry) !== JSON.stringify(imported)) throw new Error('CLI retry did not replay the outcome');
  const envelope = path.join(output, 'request.json');
  fs.writeFileSync(envelope, JSON.stringify(request, null, 2));
  if (JSON.stringify(cli(['request', envelope])) !== JSON.stringify(imported)) {
    throw new Error('JSON request retry differs');
  }
  const after = await call({ command: 'status', project: locator });
  const notes = (after.extensions['avw.reviews'] ?? []).filter(note => note.draftId === draft.draftId);
  if (after.revision !== revision + 1 || notes.length !== draft.items.length) throw new Error('Notes were not saved atomically');
  for (const item of draft.items) {
    const note = notes.find(note => note.draftItemId === item.id);
    if (!note || note.text !== item.text || note.actor !== item.actor
        || note.atMs !== item.atMs || note.endMs !== item.endMs
        || note.artifactRevision !== draft.artifactRevision || note.artifactSha256 !== draft.artifactSha256) {
      throw new Error('Saved feedback differs from the authored frozen reference');
    }
  }
  const decisions = (after.extensions['avw.reviewDecisions'] ?? []).filter(decision => decision.draftId === draft.draftId);
  if (draft.decision) {
    if (decisions.length !== 1 || decisions[0].status !== draft.decision.status || decisions[0].actor !== draft.decision.actor
        || decisions[0].artifactRevision !== draft.artifactRevision || decisions[0].artifactSha256 !== draft.artifactSha256
        || decisions[0].jobId !== draft.jobId || imported.decision.decisionId !== decisions[0].id) {
      throw new Error('Saved version decision differs from the authored frozen reference');
    }
    const reviews = await call({ command: 'reviews', project: locator, sequence: decisions[0].sequenceId });
    if (!reviews.decisions.some(decision => decision.id === decisions[0].id)) {
      throw new Error('Review decision is missing from the sequence review report');
    }
  } else if (decisions.length !== 0 || imported.decision !== undefined) {
    throw new Error('A decision appeared without creator input');
  }
  const changed = structuredClone(request);
  if (changed.draft.decision) {
    changed.draft.decision.status = changed.draft.decision.status === 'approved' ? 'changes-requested' : 'approved';
  } else changed.draft.items[0].text += ' changed';
  if ((await call(changed, false)).code !== 'E_IDEMPOTENCY_CONFLICT') throw new Error('Changed replay did not conflict');
  const artifact = await call({ command: 'artifact', project: locator, id: draft.jobId });
  const originalBytes = fs.readFileSync(artifact.path);
  try {
    fs.writeFileSync(artifact.path, 'tampered generated artifact');
    const nextDraft = structuredClone(draft);
    nextDraft.draftId = 'f26c0622-8d37-4ada-86a1-ddbd2b77d450';
    const error = await call({ ...request, key: 'browser-feedback-tamper', expectedRevision: revision + 1, draft: nextDraft }, false);
    if (!error.message.includes('bytes changed')) throw new Error('Altered artifact was accepted');
    if (JSON.stringify(await call(request)) !== JSON.stringify(imported)) throw new Error('Replay required altered media');
  } finally {
    fs.writeFileSync(artifact.path, originalBytes);
  }
  if ((await call({ command: 'status', project: locator })).revision !== revision + 1) {
    throw new Error('A failed import changed project history');
  }
  let laterDecisionRecorded = false;
  if (draft.decision) {
    const later = structuredClone(draft);
    later.items = [];
    later.draftId = 'ab14a640-15a4-4b08-baca-b2922bdf7b88';
    if (later.draftId === draft.draftId) later.draftId = 'db14a640-15a4-4b08-baca-b2922bdf7b88';
    later.decision.status = later.decision.status === 'approved' ? 'changes-requested' : 'approved';
    const laterRequest = { ...request, expectedRevision: revision + 1, key: 'browser-feedback-later-decision', draft: later };
    const recorded = await call(laterRequest);
    if (recorded.outcome.revision !== revision + 2 || recorded.decision.status !== later.decision.status
        || JSON.stringify(await call(laterRequest)) !== JSON.stringify(recorded)
        || JSON.stringify(await call(request)) !== JSON.stringify(imported)) {
      throw new Error('Later decision did not retain its own idempotent history');
    }
    const reviewed = await call({ command: 'reviews', project: locator, sequence: decisions[0].sequenceId });
    const bound = reviewed.decisions.filter(item => item.jobId === draft.jobId && item.artifactSha256 === draft.artifactSha256);
    if (!bound.some(item => item.id === imported.decision.decisionId && item.status === draft.decision.status)
        || !bound.some(item => item.id === recorded.decision.decisionId && item.status === later.decision.status)) {
      throw new Error('Both artifact-bound decisions were not retained');
    }
    laterDecisionRecorded = true;
  }
  const summary = { passed: true, browserGeneratedDraft: true, namedCliJsonAndOfficialMcp: true,
    notes: notes.length, frozenRevision: draft.artifactRevision, importedRevision: revision + 1,
    exactAuthoredTextPreserved: true, decisionRecorded: Boolean(draft.decision), laterDecisionRecorded,
    idempotentReplay: true, changedRetryConflicts: true,
    tamperedMasterRejected: true, replaySurvivesTamper: true };
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify(summary, null, 2) + '\n');
  console.log('Browser feedback import passed:', path.join(output, 'summary.json'));
} catch (error) {
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify({ passed: false, error: String(error) }, null, 2) + '\n');
  throw error;
} finally {
  await client.close();
}
