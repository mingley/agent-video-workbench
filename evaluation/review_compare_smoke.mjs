// Build a real two-version review bundle with hostile titles, then exercise
// the offline A/B players, creator decisions and notes in Chromium.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [avw, ffmpeg, ffprobe, chromium, output, playwrightRoot] = process.argv.slice(2);
if (!playwrightRoot) throw new Error('Usage: review_compare_smoke.mjs AVW FFMPEG FFPROBE CHROMIUM NEW_ROOT PLAYWRIGHT_ROOT');
const root = path.resolve(output);
fs.mkdirSync(root, { recursive: false });
const project = path.join(root, 'project');
const source = path.join(root, 'source.mp4');
const ledger = [];
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const save = () => fs.writeFileSync(path.join(root, 'commands.json'), JSON.stringify(ledger, null, 2) + '\n');
function run(argv, input) {
  const result = spawnSync(argv[0], argv.slice(1), {input, encoding:'utf8', timeout:300000});
  ledger.push({argv, status:result.status, stdout:result.stdout, stderr:result.stderr});
  save();
  if (result.status !== 0) throw new Error(result.stderr || result.stdout || String(result.error));
  return result.stdout;
}
function call(command, fields = {}) {
  const request = {command, project, ...fields};
  const response = JSON.parse(run([avw, '--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'request', '-'], JSON.stringify(request)));
  if (!response.ok) throw new Error(JSON.stringify(response));
  return response.result;
}
run([ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=3',
  '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3', '-c:v', 'libx264',
  '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
  '-c:a', 'aac', '-shortest', source]);
const sourceSha = digest(source);
call('create', {name:'Compare versions safely'});
call('import', {source, id:'source', expectedRevision:0, key:'import-generated'});
const names = [
  'First <img src=x onerror="window.unexpectedMarkup=true">',
  'Second </option><script>window.unexpectedMarkup=true</script>',
];
for (let i = 0; i < names.length; i++) {
  call('assemble', {expectedRevision:i + 1, key:`assembly-${i}`, edit:{outputId:`version_${i}`,
    name:names[i], cuts:[{id:`cut_${i}`, assetId:'source', startMs:i * 1000, endMs:i * 1000 + 2000}]}});
}
const batch = call('batch-start', {sequences:['version_0','version_1'], expectedRevision:3,
  key:'comparison-batch', noLaunch:true});
run([avw, 'worker', project]);
const status = call('batch-status', {id:batch.id});
if (!status.complete || status.succeeded !== 2) throw new Error('Generated comparison batch failed: ' + JSON.stringify(status));
const packagePath = path.join(root, 'review');
const packaged = call('delivery', {id:batch.id, destination:packagePath});
if (packaged.manifest.items.length !== 2) throw new Error('Expected two packaged versions');
const before = call('status');
run([process.execPath, path.join(process.cwd(), 'evaluation/browser_smoke.mjs'), chromium, packagePath,
  path.join(root, 'browser'), playwrightRoot]);
const browser = JSON.parse(fs.readFileSync(path.join(root, 'browser', 'summary.json')));
if (!browser.passed || !browser.independentComparisonTested || browser.results.length !== 3) {
  throw new Error('Comparison browser exercise did not finish');
}
if (digest(source) !== sourceSha || JSON.stringify(call('status')) !== JSON.stringify(before)) {
  throw new Error('Offline review comparison changed the source or project');
}
const summary = {passed:true, generatedVersions:2, hostileTitlesRenderedAsText:true,
  independentPlaybackAndSeeking:true, versionSwitchingKeepsFrozenIdentity:true,
  desktopAndPhoneWidthLayouts:true, noScriptFallback:true, sourceAndProjectUnchanged:true};
fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify(summary, null, 2) + '\n');
console.log('Review comparison passed:', path.join(root, 'summary.json'));
