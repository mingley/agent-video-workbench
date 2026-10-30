#!/usr/bin/env python3
"""Generated-media regression for protected clips and track solo semantics."""
import argparse
from array import array
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    ledger = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        assert result.returncode == 0, (argv, result.stdout, result.stderr)
        return result.stdout

    def call(command, expect_ok=True, **fields):
        request = {'command': command, **fields}
        result = subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                                 'request', '-'], input=json.dumps(request).encode(),
                                capture_output=True, timeout=180)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == expect_ok and (result.returncode == 0) == expect_ok, response
        return response['result'] if expect_ok else response['error']

    def state(project):
        return call('status', project=str(project))

    def apply(project, key, operations, expect_ok=True):
        snapshot = state(project)
        return call('apply', expect_ok=expect_ok, project=str(project), batch={
            'schemaVersion': '1.0.0', 'projectId': snapshot['projectId'],
            'baseRevision': snapshot['revision'], 'idempotencyKey': key, 'operations': operations})

    def clip(item, asset, track):
        return {'id': 'add-' + item, 'op': 'clip.add', 'params': {
            'id': item, 'asset': asset, 'track': track, 'at': t(0),
            'sourceIn': t(0), 'duration': t(60), 'fit': 'contain'}}

    def pixel(path):
        pixels = run([args.ffmpeg, '-v', 'error', '-ss', '1', '-i', path,
                      '-frames:v', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'])
        index = (320 * 360 + 180) * 3
        return list(pixels[index:index + 3])

    def audio(path):
        samples = array('f', run([args.ffmpeg, '-v', 'error', '-i', path, '-vn', '-ac', '1',
                                  '-ar', '48000', '-f', 'f32le', '-']))
        if sys.byteorder != 'little':
            samples.byteswap()
        return samples

    def tone(samples, hz):
        n = len(samples)
        return abs(sum(sample * complex(math.cos(2 * math.pi * hz * i / 48000),
                                        math.sin(2 * math.pi * hz * i / 48000))
                       for i, sample in enumerate(samples))) / n

    t = lambda n: {'value': n, 'rate': {'numerator': 30, 'denominator': 1}}
    sources = {}
    for name, color, hz in [('red', 'red', 440), ('blue', 'blue', 880)]:
        source = root / f'{name}.mp4'
        run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
             f'color=c={color}:s=320x180:r=30:d=2', '-f', 'lavfi', '-i',
             f'sine=frequency={hz}:sample_rate=48000:duration=2', '-c:v', 'libx264',
             '-preset', 'ultrafast', '-color_primaries', 'bt709', '-color_trc', 'bt709',
             '-colorspace', 'bt709', '-color_range', 'tv', '-c:a', 'aac', '-shortest', source])
        sources[name] = {'path': source, 'sha256': hashlib.sha256(source.read_bytes()).hexdigest()}

    project = root / 'protected'
    call('create', project=str(project), name='Protected disabled solos')
    call('import', project=str(project), source=str(sources['red']['path']), id='red',
         expectedRevision=0, key='import-red')
    apply(project, 'add-protected-content', [
        clip('picture', 'red', 'track_v1'),
        {'id': 'disable-embedded-audio', 'op': 'item.set', 'target': 'picture',
         'params': {'property': 'audio.enabled', 'value': False}},
        clip('voice', 'red', 'track_a1'),
    ])
    call('protect', project=str(project), expectedRevision=2, key='protect-red', range={
        'id': 'important', 'assetId': 'red', 'sequenceId': 'seq_main', 'start': t(0), 'end': t(60)})
    apply(project, 'add-disabled-solos', [
        {'id': 'add-hidden-video', 'op': 'track.add', 'params': {'id': 'hidden-video',
            'sequence': 'seq_main', 'type': 'video'}},
        {'id': 'disable-video', 'op': 'track.set', 'target': 'hidden-video',
         'params': {'enabled': False, 'solo': True}},
        {'id': 'add-hidden-audio', 'op': 'track.add', 'params': {'id': 'hidden-audio',
            'sequence': 'seq_main', 'type': 'audio'}},
        {'id': 'disable-audio', 'op': 'track.set', 'target': 'hidden-audio',
         'params': {'enabled': False, 'solo': True}},
    ])
    before = state(project)
    render = call('render', project=str(project), sequence='seq_main')
    assert state(project) == before, 'render changed the stored project'
    color = pixel(render['path'])
    samples = audio(render['path'])
    assert color[0] > 180 and color[1] < 50 and color[2] < 50, color
    assert len(samples) > 48000 and tone(samples, 440) > 0.02, 'protected speech-like tone lost'
    manifest = json.loads(Path(render['manifest']).read_text())
    saved_tracks = {t['id']: t for t in manifest['snapshot']['sequences'][0]['tracks']}
    assert not saved_tracks['hidden-video']['enabled'] and saved_tracks['hidden-video']['solo']
    assert not saved_tracks['hidden-audio']['enabled'] and saved_tracks['hidden-audio']['solo']

    # Text and caption solos suppress video in the upstream compositor. They
    # must therefore be rejected while they would hide a protected clip.
    for kind in ('text', 'caption'):
        snapshot = state(project)
        error = apply(project, 'refuse-' + kind, [
            {'id': 'add', 'op': 'track.add', 'params': {'id': 'solo-' + kind,
                'sequence': 'seq_main', 'type': kind}},
            {'id': 'solo', 'op': 'track.set', 'target': 'solo-' + kind, 'params': {'solo': True}},
        ], expect_ok=False)
        assert 'protected source range' in error['message'] and state(project) == snapshot

    # Simulate a snapshot written by an older version that accepted this state.
    # Current render must revalidate it even without any intervening edit.
    apply(project, 'legacy-caption-setup', [
        {'id': 'add', 'op': 'track.add', 'params': {'id': 'legacy-caption',
            'sequence': 'seq_main', 'type': 'caption'}},
        {'id': 'disabled', 'op': 'track.set', 'target': 'legacy-caption',
         'params': {'enabled': False, 'solo': True}},
    ])
    with sqlite3.connect(project / 'project.sqlite') as db:
        revision, raw = db.execute('SELECT revisions.id,snapshot FROM revisions '
                                   'JOIN head ON revisions.id=head.revision').fetchone()
        old_snapshot = json.loads(raw)
        for track in old_snapshot['sequences'][0]['tracks']:
            if track['id'] == 'legacy-caption':
                track['enabled'] = True
                break
        else:
            raise AssertionError('legacy track missing')
        db.execute('UPDATE revisions SET snapshot=? WHERE id=?',
                   (json.dumps(old_snapshot, separators=(',', ':')), revision))
    error = call('render', expect_ok=False, project=str(project), sequence='seq_main')
    assert 'protected source range' in error['message'], error

    # Enabled visual and audio solos remain independent and select their own
    # tracks. 440 Hz audio must survive the blue video track's solo.
    other = root / 'enabled'
    call('create', project=str(other), name='Enabled solos')
    for revision, name in enumerate(('red', 'blue')):
        call('import', project=str(other), source=str(sources[name]['path']), id=name,
             expectedRevision=revision, key='import-' + name)
    apply(other, 'enabled-solo-selection', [
        clip('red-picture', 'red', 'track_v1'),
        {'id': 'disable-red-audio', 'op': 'item.set', 'target': 'red-picture',
         'params': {'property': 'audio.enabled', 'value': False}},
        clip('red-audio', 'red', 'track_a1'),
        {'id': 'red-audio-solo', 'op': 'track.set', 'target': 'track_a1', 'params': {'solo': True}},
        {'id': 'add-blue-track', 'op': 'track.add', 'params': {'id': 'blue-track',
            'sequence': 'seq_main', 'type': 'video'}},
        clip('blue-picture', 'blue', 'blue-track'),
        {'id': 'disable-blue-audio', 'op': 'item.set', 'target': 'blue-picture',
         'params': {'property': 'audio.enabled', 'value': False}},
        {'id': 'blue-video-solo', 'op': 'track.set', 'target': 'blue-track', 'params': {'solo': True}},
        {'id': 'add-other-audio', 'op': 'track.add', 'params': {'id': 'other-audio',
            'sequence': 'seq_main', 'type': 'audio'}},
        clip('blue-audio', 'blue', 'other-audio'),
    ])
    selected = call('render', project=str(other), sequence='seq_main')
    selected_pixel = pixel(selected['path'])
    selected_samples = audio(selected['path'])
    assert selected_pixel[2] > 180 and selected_pixel[0] < 50 and selected_pixel[1] < 50, selected_pixel
    chosen, excluded = tone(selected_samples, 440), tone(selected_samples, 880)
    assert chosen > 0.02 and chosen > excluded * 5, (chosen, excluded)
    for entry in sources.values():
        assert hashlib.sha256(entry['path'].read_bytes()).hexdigest() == entry['sha256']
    summary = {'passed': True, 'protectedPixel': color, 'protectedTone440': tone(samples, 440),
               'disabledVideoAndAudioSolosIgnored': True, 'storedSoloFlagsPreserved': True,
               'hiddenProtectedContentRefused': ['text', 'caption', 'older snapshot at render'],
               'enabledSoloPixel': selected_pixel, 'enabledSoloTone440': chosen,
               'excludedTone880': excluded, 'sourcesUnchanged': True}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('Solo/protection regression passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
