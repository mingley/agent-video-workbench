#!/usr/bin/env python3
"""Measure opt-in per-item noise reduction and A/V sync on generated media."""
import argparse
from array import array
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / 'project'
    source = root / 'generated-noisy-source.mp4'
    ledger = []

    def run(argv, input_data=None, expect_ok=True):
        result = subprocess.run(list(map(str, argv)), input=input_data, capture_output=True, timeout=180)
        if (result.returncode == 0) != expect_ok:
            raise AssertionError((argv, result.returncode, result.stdout[-4000:], result.stderr[-4000:]))
        return result.stdout

    def call(command, expect_ok=True, backend=None, **fields):
        request = {'command': command, 'project': str(project), **fields}
        ffmpeg, ffprobe = backend or (args.ffmpeg, args.ffprobe)
        result = run([args.avw, '--ffmpeg', ffmpeg, '--ffprobe', ffprobe, 'request', '-'],
                     json.dumps(request, ensure_ascii=False).encode(), expect_ok)
        response = json.loads(result)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2, ensure_ascii=False) + '\n')
        assert response['ok'] == expect_ok, response
        return response['result'] if expect_ok else response['error']

    def digest(file):
        return hashlib.sha256(Path(file).read_bytes()).hexdigest()

    def audio(file, start, duration, sample_rate=48000):
        raw = run([args.ffmpeg, '-v', 'error', '-ss', str(start), '-i', file, '-t', str(duration),
                   '-vn', '-ac', '1', '-ar', str(sample_rate), '-f', 'f32le', '-'])
        values = array('f')
        values.frombytes(raw)
        if sys.byteorder != 'little':
            values.byteswap()
        assert values, 'decoded audio is empty'
        return values

    def rms(values):
        return math.sqrt(sum(value * value for value in values) / len(values))

    def tone(values, frequency, sample_rate=48000):
        real = sum(value * math.cos(2 * math.pi * frequency * i / sample_rate) for i, value in enumerate(values))
        imag = sum(value * math.sin(2 * math.pi * frequency * i / sample_rate) for i, value in enumerate(values))
        return 2 * math.hypot(real, imag) / len(values)

    def onset(file):
        samples = audio(file, 0, 2, 16000)
        window = 160
        for start in range(0, len(samples) - window, window):
            if rms(samples[start:start + window]) > 0.05:
                return start / 16000
        raise AssertionError('generated tone onset missing')

    def flash(file):
        frames = run([args.ffmpeg, '-v', 'error', '-i', file,
                      '-vf', 'crop=2:2:iw/2:ih/2,scale=1:1:flags=area,format=gray',
                      '-an', '-f', 'rawvideo', '-pix_fmt', 'gray', '-'])
        positions = [i / 30 for i, value in enumerate(frames) if value > 200]
        assert positions, 'generated video flash missing'
        return positions[0]

    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         "color=c=black:s=320x180:r=30:d=4,drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='between(t,1,1.2)'",
         '-f', 'lavfi', '-i', 'anoisesrc=color=white:amplitude=0.03:duration=4:sample_rate=48000:seed=42',
         '-f', 'lavfi', '-i', 'sine=frequency=440:duration=4:sample_rate=48000',
         '-filter_complex', "[2:a]volume='if(between(t,1,3),1,0)':eval=frame[tone];[1:a][tone]amix=inputs=2:normalize=0[a]",
         '-map', '0:v', '-map', '[a]', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
         '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709', '-color_primaries', 'bt709',
         '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv', '-c:a', 'aac', '-b:a', '192k',
         '-shortest', source])
    source_sha = digest(source)
    call('create', name='Generated per-item audio cleanup')
    call('import', source=str(source), id='source', expectedRevision=0, key='source-import')
    project_state = call('status')
    rate = {'numerator': 30, 'denominator': 1}
    time = lambda value: {'value': value, 'rate': rate}
    call('apply', batch={'schemaVersion': '1.0.0', 'projectId': project_state['projectId'],
         'baseRevision': 1, 'idempotencyKey': 'two-repeat-clips', 'operations': [
             {'id': 'first', 'op': 'clip.add', 'params': {'id': 'first', 'asset': 'source', 'track': 'track_v1',
                  'at': time(0), 'sourceIn': time(0), 'duration': time(120), 'fit': 'contain'}},
             {'id': 'second', 'op': 'clip.add', 'params': {'id': 'second', 'asset': 'source', 'track': 'track_v1',
                  'at': time(120), 'sourceIn': time(0), 'duration': time(120), 'fit': 'contain'}}]})
    baseline = call('render', sequence='seq_main')
    baseline_hash = digest(baseline['path'])
    queued_before = call('render-start', sequence='seq_main', expectedRevision=2,
                         key='queue-before-cleanup', noLaunch=True)
    request = {'action': 'set', 'sequence': 'seq_main', 'itemId': 'first',
               'settings': {'noiseReductionDb': 12, 'noiseFloorDb': -35}}
    edit_file = root / 'cleanup.json'
    edit_file.write_text(json.dumps(request, indent=2) + '\n')
    named = [args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe, 'audio-cleanup', str(project),
             '--request', str(edit_file), '--expected-revision', '2', '--key', 'cleanup-first', '--dry-run']
    preview = json.loads(run(named))
    assert preview['ok'] and preview['result']['revision'] == 3, preview
    assert call('status')['revision'] == 2
    applied = call('audio-cleanup', expectedRevision=2, key='cleanup-first', edit=request)
    assert applied == preview['result']
    named.remove('--dry-run')
    replay = json.loads(run(named))
    assert replay['ok'] and replay['result'] == applied
    run([args.avw, 'worker', project])
    earlier_job = call('job-status', id=queued_before['id'])
    assert earlier_job['state'] == 'succeeded', earlier_job
    earlier_manifest = json.loads(Path(earlier_job['result']['manifest']).read_text())
    assert earlier_manifest['revision'] == 2 and earlier_manifest['audioCleanup'] == [], earlier_manifest
    queued_after = call('render-start', sequence='seq_main', expectedRevision=3,
                        key='queue-with-cleanup', noLaunch=True)
    run([args.avw, 'worker', project])
    cleaned_job = call('job-status', id=queued_after['id'])
    assert cleaned_job['state'] == 'succeeded', cleaned_job
    cleaned = cleaned_job['result']
    manifest = json.loads(Path(cleaned['manifest']).read_text())
    assert len(manifest['audioCleanup']) == 1 and manifest['audioCleanup'][0]['itemId'] == 'first'
    assert manifest['audioCleanup'][0]['status'] == 'applied', manifest['audioCleanup']
    quiet_before = rms(audio(baseline['path'], .4, .5))
    quiet_after = rms(audio(cleaned['path'], .4, .5))
    tone_before = tone(audio(baseline['path'], 1.5, 1), 440)
    tone_after = tone(audio(cleaned['path'], 1.5, 1), 440)
    sibling_before = rms(audio(baseline['path'], 4.4, .5))
    sibling_after = rms(audio(cleaned['path'], 4.4, .5))
    assert quiet_after < quiet_before * .75, (quiet_before, quiet_after)
    assert .85 < tone_after / tone_before < 1.15, (tone_before, tone_after)
    assert .9 < sibling_after / sibling_before < 1.1, (sibling_before, sibling_after)
    onset_before, onset_after, flash_after = onset(baseline['path']), onset(cleaned['path']), flash(cleaned['path'])
    assert abs(onset_after - onset_before) < 1/30 and abs(onset_after - flash_after) < .05, (onset_before, onset_after, flash_after)

    # The source-preserving path must never silently ignore a configured edit.
    call('assemble', expectedRevision=3, key='plain-preserving', edit={'outputId': 'plain', 'name': 'Plain',
         'cuts': [{'id': 'cut', 'assetId': 'source', 'startMs': 0, 'endMs': 4000}]})
    refused = call('audio-cleanup', expect_ok=False, expectedRevision=4, key='refuse-preserving', edit={
        **request, 'sequence': 'plain', 'itemId': 'plain_cut'})
    assert 'source-preserving' in refused['message'], refused
    assert call('status')['revision'] == 4

    # An unavailable filter fails before publication, retaining earlier output.
    fake = root / 'ffmpeg-without-afftdn'
    fake.write_text('#!' + sys.executable + '\nimport os,sys\n'
                    "if '-filters' in sys.argv: print('Filters:\\n T.. other V->V test');sys.exit(0)\n"
                    f'os.execv({os.path.realpath(args.ffmpeg)!r}, [{os.path.realpath(args.ffmpeg)!r}] + sys.argv[1:])\n')
    fake.chmod(0o755)
    error = call('render', expect_ok=False, backend=(str(fake), args.ffprobe), sequence='seq_main')
    assert 'lacks the afftdn filter' in error['message'], error
    assert digest(baseline['path']) == baseline_hash

    # Disabling a configured item is explicitly reported, not silently applied
    # to the remaining source alias. It can still be removed while disabled.
    state = call('status')
    call('apply', batch={'schemaVersion': '1.0.0', 'projectId': state['projectId'], 'baseRevision': 4,
         'idempotencyKey': 'disable-cleanup-item', 'operations': [
             {'id': 'disable', 'op': 'item.set', 'target': 'first', 'params': {'property': 'enabled', 'value': False}}]})
    skipped = call('render', sequence='seq_main')
    skipped_manifest = json.loads(Path(skipped['manifest']).read_text())
    assert skipped_manifest['audioCleanup'][0]['status'] == 'skipped', skipped_manifest['audioCleanup']
    call('audio-cleanup', expectedRevision=5, key='cleanup-remove', edit={'action': 'remove', 'sequence': 'seq_main', 'itemId': 'first'})
    call('restore', revision=2, expectedRevision=6, key='restore-original-edit')
    restored = call('render', sequence='seq_main')
    restored_quiet = rms(audio(restored['path'], .4, .5))
    assert abs(restored_quiet / quiet_before - 1) < .05, (quiet_before, restored_quiet)
    assert digest(source) == source_sha and digest(baseline['path']) == baseline_hash
    state = call('status')
    asset = next(asset for asset in state['assets'] if asset['id'] == 'source')
    assert digest(project / asset['uri']) == source_sha
    summary = {'passed': True, 'noiseRmsBefore': quiet_before, 'noiseRmsAfter': quiet_after,
               'toneAmplitudeRatio': tone_after / tone_before, 'unselectedNoiseRatio': sibling_after / sibling_before,
               'baselineToneOnsetSeconds': onset_before, 'cleanedToneOnsetSeconds': onset_after,
               'cleanedFlashSeconds': flash_after, 'measuredOnsetShiftSeconds': onset_after - onset_before,
               'frozenJobsKeepCleanupState': True, 'namedCliJsonParityAndReplay': True,
               'sourcePreservingAssemblyRefuses': True, 'missingFilterRefuses': True,
               'disabledItemReportedSkipped': True, 'removalAndRestore': True,
               'originalAndEarlierArtifactUnchanged': True,
               'limitations': ['Generated sine and white-noise fixture; no human speech, music-quality or arbitrary-noise claim']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('Audio cleanup passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
