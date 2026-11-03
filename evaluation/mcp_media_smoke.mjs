// Exercise model-visible image/audio blocks with the official MCP SDK and
// generated media. A host still decides whether its model can use the blocks.
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: mcp_media_smoke.mjs AVW FFMPEG FFPROBE NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot, 'dist/esm/client/stdio.js')));
fs.mkdirSync(root, { recursive: false });
const source = path.join(root, 'source.mp4');
const ledger = [];
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const canonical = value => Array.isArray(value) ? value.map(canonical)
  : value && typeof value === 'object'
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const same = (left, right) => JSON.stringify(canonical(left)) === JSON.stringify(canonical(right));
function run(program, args, options = {}) {
  const result = spawnSync(program, args, { encoding: 'utf8', timeout: 180000, ...options });
  if (result.status !== 0) throw new Error(`${program}: ${result.stderr}`);
  return result.stdout;
}
run(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=160x90:r=10:d=4',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=4', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709',
  '-colorspace', 'bt709', '-color_range', 'tv', '-c:a', 'aac', '-shortest', source]);
const original = sha(fs.readFileSync(source));
const client = new Client({ name: 'avw-media-content-conformance', version: '1.0.0' });
const transport = new StdioClientTransport({ command: avw,
  args: ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'mcp', '--root', root], stderr: 'inherit' });
try {
  await client.connect(transport);
  const tools = await client.listTools();
  if (!tools.tools.some(tool => tool.name === 'avw-media')) throw new Error('Media tool is not discoverable');
  async function call(name, args, expectOk = true) {
    const response = await client.callTool({ name, arguments: args });
    const envelope = response.structuredContent ?? JSON.parse(response.content[0].text);
    const recorded = structuredClone(envelope);
    if (name === 'avw' && args.command === 'agent-media' && recorded.ok) {
      recorded.result.content.data = '[base64 omitted from ledger]';
    }
    ledger.push({ name, args, response: { envelope: recorded, contentTypes: response.content.map(c => c.type) } });
    fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2));
    if (envelope.ok !== expectOk || Boolean(response.isError) === expectOk) throw new Error(JSON.stringify(envelope));
    return { result: expectOk ? envelope.result : envelope.error, response };
  }
  const avwCall = async args => (await call('avw', args)).result;
  let parityIndex = 0;
  async function assertParity(args, native) {
    const viaService = await call('avw', { command: 'agent-media', media: args });
    const nativeContent = native.response.content.find(c => c.type === 'image' || c.type === 'audio');
    const expected = { metadata: native.result, content: nativeContent };
    if (!same(viaService.result, expected)) {
      throw new Error('MCP native media and shared service results differ');
    }
    const mediaFile = path.join(root, `media-${parityIndex++}.json`);
    fs.writeFileSync(mediaFile, JSON.stringify(args));
    const envelopeFile = path.join(root, `media-envelope-${parityIndex}.json`);
    fs.writeFileSync(envelopeFile, JSON.stringify({ command: 'agent-media', media: args }));
    for (const command of [['agent-media', '--request', mediaFile], ['request', envelopeFile]]) {
      const output = run(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root, ...command],
        { maxBuffer: 8 * 1024 * 1024 });
      const envelope = JSON.parse(output);
      if (!envelope.ok || !same(envelope.result, expected)) {
        throw new Error('Named/JSON CLI media result differs from MCP');
      }
    }
  }
  await avwCall({ command: 'create', project: 'project', name: 'Model-visible media' });
  await avwCall({ command: 'import', project: 'project', source: 'source.mp4', id: 'source',
    expectedRevision: 0, key: 'import-source' });
  const frameRequest = { kind: 'frame', project: 'project', assetId: 'source', atMs: 500 };
  const frame = await call('avw-media', frameRequest);
  await assertParity(frameRequest, frame);
  const frameBlock = frame.response.content.find(c => c.type === 'image');
  const frameBytes = Buffer.from(frameBlock.data, 'base64');
  if (frameBlock.mimeType !== 'image/png' || sha(frameBytes) !== frame.result.sha256
      || frame.result.requestedSourceTimeMs !== 500 || frame.result.assetId !== 'source') {
    throw new Error('Frame block did not match verified evidence');
  }
  const framePath = path.join(root, 'frame.png');
  fs.writeFileSync(framePath, frameBytes);
  const frameProbe = JSON.parse(run(ffprobe, ['-v', 'error', '-show_streams', '-of', 'json', framePath]));
  if (!frameProbe.streams.some(s => s.codec_name === 'png' && s.width > 0 && s.height > 0)) {
    throw new Error('MCP image bytes are not a decodable PNG');
  }
  await avwCall({ command: 'assemble', project: 'project', expectedRevision: 1, key: 'assemble-source',
    edit: { outputId: 'natural', name: 'Natural', cuts: [
      { id: 'cut', assetId: 'source', startMs: 0, endMs: 4000 },
    ] } });
  const render = await avwCall({ command: 'render-start', project: 'project', sequence: 'natural',
    expectedRevision: 2, key: 'render-source', noLaunch: true });
  const proxy = await avwCall({ command: 'analyze-start', project: 'project', expectedRevision: 2,
    key: 'source-proxy', noLaunch: true,
    task: { kind: 'proxy', assetId: 'source', startMs: 1000, endMs: 3000, width: 320, height: 180 } });
  run(avw, ['worker', path.join(root, 'project')]);
  for (const job of [render, proxy]) {
    const result = await avwCall({ command: 'job-status', project: 'project', id: job.id });
    if (result.state !== 'succeeded') throw new Error('Generated job failed: ' + JSON.stringify(result));
  }
  const sheetRequest = { kind: 'contact-sheet', project: 'project', jobId: render.id };
  const sheet = await call('avw-media', sheetRequest);
  await assertParity(sheetRequest, sheet);
  const sheetBlock = sheet.response.content.find(c => c.type === 'image');
  const sheetBytes = Buffer.from(sheetBlock.data, 'base64');
  if (sha(sheetBytes) !== sheet.result.sha256 || sheet.result.jobId !== render.id || sheet.result.revision !== 2) {
    throw new Error('Contact sheet lost artifact identity or hash');
  }
  const audioRequest = { kind: 'audio-proxy', project: 'project', jobId: proxy.id, startMs: 250, durationMs: 1000 };
  const audio = await call('avw-media', audioRequest);
  await assertParity(audioRequest, audio);
  const audioBlock = audio.response.content.find(c => c.type === 'audio');
  const wav = Buffer.from(audioBlock.data, 'base64');
  if (audioBlock.mimeType !== 'audio/wav' || sha(wav) !== audio.result.sha256
      || audio.result.requestedSourceStartMs !== 1250 || audio.result.requestedSourceEndMs !== 2250
      || audio.result.sampleRateHz !== 16000 || audio.result.sampleCount !== 16000
      || !audio.result.returnedFullRequestedDuration) throw new Error('Audio block timing or identity mismatch');
  const audioPath = path.join(root, 'audio.wav');
  fs.writeFileSync(audioPath, wav);
  const probe = JSON.parse(run(ffprobe, ['-v', 'error', '-show_streams', '-of', 'json', audioPath]));
  if (!probe.streams.some(s => s.codec_name === 'pcm_s16le' && s.sample_rate === '16000' && s.channels === 1)) {
    throw new Error('MCP audio is not a decodable mono WAV');
  }
  const pcm = spawnSync(ffmpeg, ['-v', 'error', '-i', audioPath, '-f', 's16le', '-'], { timeout: 30000 });
  if (pcm.status !== 0 || pcm.stdout.length !== 32000) throw new Error('Audio decode has wrong duration');
  let peak = 0;
  for (let i = 0; i < pcm.stdout.length; i += 2) peak = Math.max(peak, Math.abs(pcm.stdout.readInt16LE(i)));
  if (peak < 100) throw new Error('Generated tone was not present in extracted audio');
  const artifact = await avwCall({ command: 'artifact', project: 'project', id: proxy.id });
  const proxyPath = artifact.attachments.find(file => file.path.endsWith('/proxy.mp4')).path;
  const proxyBytes = fs.readFileSync(proxyPath);
  try {
    fs.writeFileSync(proxyPath, 'altered proxy');
    const rejected = await call('avw-media', { kind: 'audio-proxy', project: 'project', jobId: proxy.id,
      startMs: 0, durationMs: 1000 }, false);
    if (!rejected.result.message.includes('changed')) throw new Error('Tampered proxy was not rejected');
  } finally { fs.writeFileSync(proxyPath, proxyBytes); }
  const unsafe = await call('avw-media', { kind: 'contact-sheet', project: 'project', jobId: render.id,
    path: '/etc/passwd' }, false);
  if (unsafe.response.content.some(c => c.type === 'image' || c.type === 'audio')) throw new Error('Unsafe request returned media');
  const invalidAudio = { ...audioRequest, durationMs: 15001 };
  const nativeError = await call('avw-media', invalidAudio, false);
  const serviceError = await call('avw', { command: 'agent-media', media: invalidAudio }, false);
  if (!same(nativeError.result, serviceError.result)) throw new Error('MCP error envelopes differ');
  const invalidFile = path.join(root, 'invalid-media.json');
  fs.writeFileSync(invalidFile, JSON.stringify(invalidAudio));
  const invalidCli = spawnSync(avw, ['--ffmpeg', ffmpeg, '--ffprobe', ffprobe, '--workspace', root,
    'agent-media', '--request', invalidFile], { encoding: 'utf8', timeout: 30000 });
  if (invalidCli.status === 0 || !same(JSON.parse(invalidCli.stdout).error, nativeError.result)) {
    throw new Error('Named CLI media error differs from MCP');
  }
  if (sha(fs.readFileSync(source)) !== original) throw new Error('Original source was changed');
  fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify({ passed: true, officialMcpSdk: true,
    imageAndAudioBlocksDecoded: true, namedJsonAndMcpParity: true, matchingErrorEnvelopes: true,
    sourcePreviewSamples: audio.result.sampleCount,
    requestedSourceIntervalMs: [1250, 2250], verifiedContactSheet: true, alteredProxyRejected: true,
    arbitraryPathRejected: true, sourceUnchanged: true,
    limitation: 'Client/model image and audio modality support is host dependent.' }, null, 2));
  console.log('MCP media-content workflow passed:', path.join(root, 'summary.json'));
} finally { await client.close(); }
