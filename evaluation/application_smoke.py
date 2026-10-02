#!/usr/bin/env python3
"""Fresh-process application regression on generated, redistributable media."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'font', 'output'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / 'project'
    ledger = []

    def run(argv, expected=0):
        p = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        ledger.append({'argv': list(map(str, argv)), 'exit': p.returncode,
                       'stdout': p.stdout.decode(errors='replace'),
                       'stderr': p.stderr.decode(errors='replace')})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert p.returncode == expected, ledger[-1]
        return p.stdout

    def avw(*argv, expected=0):
        result = json.loads(run([args.avw, '--ffmpeg', args.ffmpeg,
                                '--ffprobe', args.ffprobe, *argv], expected))
        assert result['ok'] == (expected == 0), result
        return result.get('result', result)

    def state():
        return avw('status', project)

    def time(frames):
        return {'value': frames, 'rate': {'numerator': 30, 'denominator': 1}}

    def op(name, params, target=None):
        return {'id': str(len(params)) + '-' + name + '-' + str(target),
                'op': name, 'params': params, 'target': target}

    def apply(key, operations, expected=0):
        s = state()
        request = {'schemaVersion': '1.0.0', 'projectId': s['projectId'],
                   'baseRevision': s['revision'], 'idempotencyKey': key,
                   'description': key, 'operations': operations}
        for i, operation in enumerate(operations):
            operation['id'] = 'op-' + str(i)
        path = root / (key + '.json')
        path.write_text(json.dumps(request))
        return avw('apply', project, '--request', path, expected=expected)

    source = root / 'source.mp4'
    # Two minutes, with a red opening, green demonstration and blue ending.
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'color=c=red:s=640x360:r=30:d=120', '-f', 'lavfi', '-i',
         'sine=frequency=440:sample_rate=48000:duration=120', '-vf',
         "drawbox=color=lime:t=fill:enable='between(t,60,90)',drawbox=color=blue:t=fill:enable='gte(t,90)'",
         '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
         '-c:a', 'aac', '-shortest', source])
    original_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    avw('doctor')
    avw('create', project, '--name', 'Synthetic creator loop')
    for source_path, asset_id in [(source, 'phone'), (Path(args.font), 'font')]:
        s = state()
        result = avw('import', project, source_path, '--id', asset_id,
                     '--expected-revision', s['revision'], '--key', 'import-' + asset_id)
        assert avw('import', project, source_path, '--id', asset_id,
                   '--expected-revision', s['revision'], '--key', 'import-' + asset_id) == result
    operations = [op('track.add', {'sequence': 'seq_main', 'id': 'captions', 'type': 'caption'})]
    for name, start, source_in in [('intro', 0, 0), ('demo', 450, 1800)]:
        for track, suffix in [('track_v1', ''), ('track_a1', '_audio')]:
            operations.append(op('clip.add', {'id': name + suffix, 'asset': 'phone',
                'track': track, 'at': time(start), 'sourceIn': time(source_in),
                'duration': time(450), 'fit': 'cover'}))
        operations.append(op('item.set', {'property': 'audio.enabled', 'value': False}, name))
        operations.append(op('caption.add', {'id': name + '_caption', 'track': 'captions',
            'at': time(start), 'duration': time(450), 'text': "Literal: 'quote', \\ slash; 100%"}))
        operations.append(op('item.set', {'property': 'text.style.fontSize', 'value': 22}, name + '_caption'))
        operations.append(op('item.set', {'property': 'text.style.fontAssetId', 'value': 'font'}, name + '_caption'))
    draft_revision = apply('draft-edit', operations)['revision']
    draft = avw('render', project)
    manifest = json.loads(Path(draft['manifest']).read_text())
    assert manifest['verification']['expectedFrames'] == 900
    assert manifest['verification']['decoded']
    # Revision restores a known source interval, changes the opening and caption style.
    operations = [op('item.move', {'to': time(480)}, name)
                  for name in ('demo', 'demo_audio', 'demo_caption')]
    for track, name in [('track_v1', 'restored'), ('track_a1', 'restored_audio')]:
        operations.append(op('clip.add', {'id': name, 'asset': 'phone', 'track': track,
            'at': time(450), 'sourceIn': time(450), 'duration': time(30), 'fit': 'cover'}))
    operations.append(op('item.set', {'property': 'audio.enabled', 'value': False}, 'restored'))
    for name in ('intro_caption', 'demo_caption'):
        operations.append(op('item.set', {'property': 'text.style.fontSize', 'value': 18}, name))
    operations.append(op('item.set', {'property': 'text.text', 'value': 'New opening'}, 'intro_caption'))
    revision = apply('restore-style-opening', operations)['revision']
    revised = avw('render', project)
    assert json.loads(Path(revised['manifest']).read_text())['verification']['expectedFrames'] == 930
    avw('restore', project, '--revision', draft_revision, '--expected-revision', revision, '--key', 'undo')
    undone = avw('render', project)
    assert hashlib.sha256(Path(undone['path']).read_bytes()).hexdigest() == hashlib.sha256(Path(draft['path']).read_bytes()).hexdigest()
    avw('restore', project, '--revision', revision, '--expected-revision', state()['revision'], '--key', 'redo')
    assert state()['sequences'] == json.loads(Path(revised['manifest']).read_text())['snapshot']['sequences']
    assert hashlib.sha256(source.read_bytes()).hexdigest() == original_hash
    # Corrupting a managed original must reject rendering, preserving earlier finals.
    managed = project / ('originals/' + original_hash)
    with managed.open('ab') as f:
        f.write(b'corruption')
    avw('render', project, expected=1)
    assert Path(draft['path']).is_file()
    (root / 'summary.json').write_text(json.dumps({'passed': True, 'draftFrames': 900,
        'revisedFrames': 930, 'sourceUnchanged': True, 'undoBytesMatch': True,
        'corruptOriginalRejected': True, 'limitations': ['synthetic SDR only; no real iPhone or hosted bot trial']}, indent=2))
    print('Application smoke passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
