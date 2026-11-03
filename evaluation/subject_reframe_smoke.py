#!/usr/bin/env python3
"""Review and render a crop path from generated subject observations."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / 'project'
    source = root / 'moving.mp4'
    ledger = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        assert result.returncode == 0, (argv, result.stdout, result.stderr)
        return result.stdout

    def call(command, **fields):
        request = {'command': command, 'project': str(project), **fields}
        result = subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                                 'request', '-'], input=json.dumps(request).encode(),
                                capture_output=True, timeout=180)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert result.returncode == 0 and response['ok'], response
        return response['result']

    def sha(path):
        return hashlib.sha256(Path(path).read_bytes()).hexdigest()

    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=0x404040:s=320x180:r=30:d=3',
         '-f', 'lavfi', '-i', 'color=c=white:s=48x48:r=30:d=3', '-filter_complex',
         "[1:v]drawbox=x=6:y=6:w=10:h=10:c=black:t=fill,drawbox=x=29:y=29:w=12:h=12:c=black:t=fill[p];[0:v][p]overlay=x='64+24*t':y=54",
         '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '14', '-pix_fmt', 'yuv420p',
         '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv', source])
    source_hash = sha(source)
    call('create', name='Subject reframe')
    call('import', source=str(source), id='source', expectedRevision=0, key='import-source')
    state = call('status')
    rate = {'numerator': 30, 'denominator': 1}
    time = lambda value: {'value': value, 'rate': rate}
    call('apply', batch={'schemaVersion': '1.0.0', 'projectId': state['projectId'],
         'baseRevision': 1, 'idempotencyKey': 'add-subject-clip', 'operations': [
             {'id': 'clip', 'op': 'clip.add', 'params': {'id': 'take', 'asset': 'source',
              'track': 'track_v1', 'at': time(0), 'sourceIn': time(0), 'duration': time(90), 'fit': 'contain'}}]})
    samples = []
    for ms in (0, 500, 1000, 1500, 2000, 2500, 2990):
        samples.append({'sourceMs': ms, 'confidence': 0.9, 'regions': [{
            'x': (64 + 24 * ms / 1000) / 320, 'y': 54 / 180, 'width': 48 / 320, 'height': 48 / 180}]})
    observations = {'schemaVersion': 1, 'itemId': 'take', 'sourceSha256': source_hash,
                    'observer': 'generated moving-box fixture', 'safeInset': 0.05,
                    'maxGapMs': 500, 'samples': samples}
    proposal = call('suggest-reframe', expectedRevision=2, observations=observations)
    assert proposal['status'] == 'review-required', proposal
    request_file = root / 'observations.json'
    request_file.write_text(json.dumps(observations))
    cli = json.loads(run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                          'suggest-reframe', project, '--expected-revision', '2', '--request', request_file]))
    assert cli['result'] == proposal
    call('studio', edit=proposal['edit'], expectedRevision=2, key='accept-reviewed-crop')
    rendered = call('render', sequence='seq_main')
    bounds = []
    for second in (0.1, 1, 2, 2.9):
        pixels = run([args.ffmpeg, '-v', 'error', '-ss', str(second), '-i', rendered['path'],
                      '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'gray', '-'])
        points = [i % 360 for i, value in enumerate(pixels) if value > 180]
        assert points, second
        left, right = min(points), max(points)
        assert left >= 10 and right <= 350, (second, left, right)
        bounds.append({'seconds': second, 'left': left, 'right': right})
    assert sha(source) == source_hash
    assert call('artifact', id=rendered['artifactId'])['verified']
    (root / 'summary.json').write_text(json.dumps({'passed': True, 'observationsAreSynthetic': True,
        'namedCliMatchesJson': True, 'renderedSubjectBounds': bounds, 'sourceUnchanged': True,
        'scope': 'sampled fixture; no claim of automatic subject detection or real-scene accuracy'}, indent=2) + '\n')
    print('Observed-subject reframe passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
