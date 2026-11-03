// First-run diagnostics through the official MCP SDK and named CLI.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: host_readiness_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
fs.mkdirSync(path.join(root, 'workspace'));
const sentinel = path.join(root, 'workspace', 'keep.txt');
fs.writeFileSync(sentinel, 'This file must not change.');
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const originalSha = sha(sentinel);
const ledger = [];
const client = new Client({ name: 'avw-host-readiness', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
function verify(result) {
  if (result.status !== 'local-checks-passed' || result.workspace.writable.status !== 'pass'
      || result.workspace.freeSpace.status !== 'measured' || !/^\d+$/.test(result.workspace.freeSpace.availableBytesDecimal)
      || result.media.status !== 'pass' || result.media.doctor.ready !== true
      || !result.media.doctor.checks.includes('full output decode')
      || result.workspace.persistence.status !== 'unknown' || result.workspace.filesystemLocking.status !== 'unknown'
      || result.host.modelImageConsumption.status !== 'unknown' || result.host.modelAudioConsumption.status !== 'unknown'
      || result.host.accountCompatibility.status !== 'unknown' || !result.capabilities.commands.includes('host-readiness')) {
    throw new Error('Readiness result is incomplete or overclaims: ' + JSON.stringify(result));
  }
}
try {
  await client.connect(transport);
  const request = { command: 'host-readiness', workspace: 'workspace' };
  const response = await client.callTool({ name: 'avw', arguments: request });
  const envelope = response.structuredContent ?? JSON.parse(response.content[0].text);
  ledger.push({ request, response: envelope });
  if (!envelope.ok) throw new Error(JSON.stringify(envelope));
  const mcp = envelope.result;
  verify(mcp);
  const named = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root,
    'host-readiness', 'workspace'], { encoding: 'utf8', timeout: 300000 });
  if (named.status !== 0) throw new Error(named.stderr || named.stdout);
  const cli = JSON.parse(named.stdout).result;
  verify(cli);
  if (cli.workspace.path !== mcp.workspace.path || cli.media.doctor.ffmpeg !== mcp.media.doctor.ffmpeg
      || JSON.stringify(cli.capabilities) !== JSON.stringify(mcp.capabilities)) {
    throw new Error('CLI/MCP diagnostics do not agree on configuration');
  }
  if (sha(sentinel) !== originalSha || JSON.stringify(fs.readdirSync(path.join(root, 'workspace'))) !== JSON.stringify(['keep.txt'])) {
    throw new Error('Readiness probe changed a pre-existing file or left its temporary file');
  }
  const outside = await client.callTool({ name: 'avw', arguments: { command: 'host-readiness', workspace: '..' } });
  const denied = outside.structuredContent ?? JSON.parse(outside.content[0].text);
  if (denied.ok || denied.error.code !== 'E_INVALID_REQUEST') throw new Error('Out-of-scope workspace accepted');
  ledger.push({ request: { command: 'host-readiness', workspace: '..' }, response: denied });
  fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2));
  fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify({ passed: true, officialMcpSdk: true,
    namedCliEquivalent: true, writeSyncAndCleanupVerified: true, diskBytesMeasured: true,
    actualFfmpegDoctorSucceeded: true, outsideRootRejected: true, sentinelUnchanged: true,
    persistenceAndModelMediaConsumptionUnknown: true,
    limitation: 'Local diagnostics do not qualify a hosted account, model media consumption, or persistence.' }, null, 2));
  console.log('Host readiness workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
