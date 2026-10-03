#!/usr/bin/env python3
"""Creator workflow qualification through fresh CLI processes and real renders."""
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

    def run(argv):
        p = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        assert p.returncode == 0, (argv, p.stdout, p.stderr)
        return p.stdout

    def call(command, **fields):
        request = {'command': command, **fields}
        p = subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                            'request', '-'], input=json.dumps(request).encode(),
                           capture_output=True, timeout=180)
        response = json.loads(p.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert p.returncode == 0 and response['ok'], response
        return response['result']

    def state():
        return call('status', project=str(project))

    def edit(action, **fields):
        revision = state()['revision']
        return call('studio', project=str(project), edit={'action': action, **fields},
                    expectedRevision=revision, key=f'{action}-{revision}')

    def apply(ops):
        p = state()
        for i, op in enumerate(ops):
            op['id'] = str(i)
        return call('apply', project=str(project), batch={
            'schemaVersion': '1.0.0', 'projectId': p['projectId'], 'baseRevision': p['revision'],
            'idempotencyKey': f'creator-edit-{p["revision"]}', 'operations': ops})

    call('create', project=str(project), name='Creator suite')
    hashes = {}
    for name, color in [('phone', 'red'), ('closeup', 'green'), ('product', 'blue')]:
        path = root / f'{name}.mp4'
        run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
             f'color=c={color}:s=640x360:r=30:d=8', '-f', 'lavfi', '-i',
             'sine=frequency=440:sample_rate=48000:duration=8', '-c:v', 'libx264',
             '-preset', 'ultrafast', '-c:a', 'aac', '-shortest', path])
        hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        call('import', project=str(project), source=str(path), id=name,
             expectedRevision=state()['revision'], key='import-' + name)
    call('import', project=str(project), source=str(Path(args.font).resolve()), id='font',
         expectedRevision=state()['revision'], key='import-font')
    for asset in ('phone', 'closeup', 'product'):
        call('transcript-import', project=str(project), expectedRevision=state()['revision'],
             key='transcript-' + asset, transcript={'assetId': asset, 'language': 'en',
             'provider': 'reviewed-fixture', 'cues': [{'id': 'sentence', 'startMs': 0,
             'endMs': 8000, 'text': 'Try the product'}]})
    call('compose', project=str(project), expectedRevision=state()['revision'], key='compose-creator',
         edit={'outputId': 'creator', 'name': 'Creator', 'fontAssetId': 'font',
               'width': 360, 'height': 640, 'fontSize': 22, 'cuts': [
                   {'id': 'a', 'assetId': 'phone', 'startMs': 0, 'endMs': 2000},
                   {'id': 'b', 'assetId': 'closeup', 'startMs': 2000, 'endMs': 4000},
                   {'id': 'c', 'assetId': 'product', 'startMs': 4000, 'endMs': 6000}]})
    base = call('render', project=str(project), sequence='creator')
    edit('review-add', id='product-note', jobId=base['artifactId'], atMs=3000,
         endMs=3500, actor='fixture-reviewer', text='Keep the closeup visible')
    edit('omit', id='first', itemId='creator_a', reason='shorter opening')
    mapping = call('reviews', project=str(project), sequence='creator')['items'][0]
    assert mapping['placements'][0]['atMs'] == 1000, mapping
    edit('omit', id='second', itemId='creator_b', reason='another sentence')
    profile = {'id': 'brand', 'version': 1, 'fontAssetId': 'font', 'fontSize': 20,
               'color': '#FFFFFFFF', 'fit': 'cover', 'loudnessLufs': -18, 'truePeakDb': -1}
    edit('profile-put', profile=profile)
    edit('profile-apply', sequence='creator', profileId='brand', version=1)
    edit('restore-omission', id='first', atMs=0)
    p = state()
    assert p['extensions']['avw.omission.second']['restored'] is False
    assert p['sequences'][1]['tracks'][1]['items'][0]['payload']['data']['style']['fontSize'] == 20
    edit('transcript-correct', assetId='phone', cues={'sentence': 'Try Mingley'}, sequences=['creator'])
    call('transcript-import', project=str(project), expectedRevision=state()['revision'],
         key='reanalysis-phone', transcript={'assetId': 'phone', 'language': 'en', 'provider': 'new-analysis',
         'cues': [{'id': 'sentence', 'startMs': 0, 'endMs': 8000, 'text': 'Wrong new name'}]})
    assert call('transcript-search', project=str(project), query='Mingley')['items']
    # A visual replacement leaves the continuous dialogue route untouched.
    before = call('render', project=str(project), sequence='creator')
    edit('layer', sequence='creator', id='broll', assetId='closeup', atMs=0,
         sourceStartMs=0, durationMs=1000, fit='cover')
    edit('replace-layer', itemId='broll', assetId='product', sourceStartMs=1000)
    after = call('render', project=str(project), sequence='creator')
    def audio(path):
        return run([args.ffmpeg, '-v', 'error', '-i', path, '-vn', '-f', 's16le', '-'])
    assert audio(before['path']) == audio(after['path']), 'B-roll changed dialogue audio'
    manifest = json.loads(Path(after['manifest']).read_text())
    assert manifest['audioQc']['status'] == 'passed', manifest['audioQc']
    assert (Path(after['path']).parent / 'captions.srt').exists()
    # Independent framing and translated text are pinned to this revision.
    frozen = state()['revision']
    edit('variant', sourceRevision=frozen, sequence='creator', id='square', name='Square', width=360, height=360)
    captions = {i['id']: 'Prueba el producto' for t in state()['sequences'][1]['tracks']
                for i in t['items'] if i['payload']['kind'] == 'caption'}
    edit('variant', sourceRevision=frozen, sequence='creator', id='landscape_es', name='Landscape Spanish',
         width=640, height=360, language='es', captions=captions)
    edit('template-put', template={'id': 'two_shots', 'version': 1, 'name': 'Two shots',
         'profileId': 'brand', 'profileVersion': 1, 'width': 360, 'height': 640,
         'slots': [{'id': 'intro', 'minDurationMs': 1000, 'maxDurationMs': 3000},
                   {'id': 'demo', 'minDurationMs': 1000, 'maxDurationMs': 3000}]})
    edit('template-instantiate', templateId='two_shots', version=1, outputId='templated', name='New template',
         bindings=[{'id': 'intro', 'assetId': 'closeup', 'startMs': 0, 'endMs': 2000},
                   {'id': 'demo', 'assetId': 'product', 'startMs': 0, 'endMs': 2000}])
    batch_revision = state()['revision']
    batch = call('batch-start', project=str(project), sequences=['creator', 'square', 'landscape_es'],
                 expectedRevision=batch_revision, key='batch-three-formats', noLaunch=True)
    apply([{'op': 'project.rename', 'params': {'name': 'Edited while batch queued'}}])
    run([args.avw, 'worker', project])
    status = call('batch-status', project=str(project), id=batch['id'])
    assert status['complete'] and status['succeeded'] == 3, status
    assert all(item['revision'] == batch_revision for item in status['items'])
    package = call('delivery', project=str(project), id=batch['id'], destination=str(root / 'delivery'))
    assert len(package['manifest']['items']) == 3
    for item in package['manifest']['items']:
        folder = root / 'delivery' / item['jobId']
        for file in item['files']:
            assert hashlib.sha256((folder / file['path']).read_bytes()).hexdigest() == file['sha256']
    library = call('library-export', project=str(project), destination=str(root / 'library'))
    assert not library['privateFootageIncluded']
    new_project = root / 'new_project'
    call('create', project=str(new_project), name='Reused brand')
    call('library-import', project=str(new_project), source=str(root / 'library'), prefix='usual',
         expectedRevision=0, key='import-brand-library')
    assert call('studio-state', project=str(new_project))['decisions']['avw.profile.usual_brand.1']['fontAssetId']
    otio = call('interchange-export', project=str(project), sequence='creator')
    assert otio['lossReport'], 'Caption/effect loss must be reported'
    call('interchange-import', project=str(project), document=otio['document'], id='roundtrip',
         expectedRevision=state()['revision'], key='import-otio-roundtrip')
    call('backup', project=str(project), destination=str(root / 'backup'))
    call('backup-restore', source=str(root / 'backup'), destination=str(root / 'restored'))
    assert call('verify-project', project=str(root / 'restored'))['complete']
    original = next((project / 'originals').glob(hashes['phone']))
    original.unlink()
    assert not call('verify-project', project=str(project))['complete']
    call('relink', project=str(project), source=str(root / 'phone.mp4'), sha256=hashes['phone'])
    assert call('verify-project', project=str(project))['complete']
    assert call('catalog', workspace=str(root), query='Creator', limit=100)['items']
    call('cache-gc', project=str(project), dryRun=True, graceSeconds=60)
    (root / 'summary.json').write_text(json.dumps({'passed': True, 'checks': [
        'selective restore with later style', 'corrections survive reanalysis',
        'review source remap', 'B-roll preserves dialogue samples', 'measured loudness',
        'versioned profile/template', 'frozen batch in three formats with Spanish captions',
        'sidecars/covers/hash-verified review package', 'verified backup restore and relink',
        'catalog and safe cache planning', 'portable reusable font/profile library', 'OTIO supported cut roundtrip with loss report']}, indent=2))
    print('Creator workflow passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
