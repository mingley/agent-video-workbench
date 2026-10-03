#!/usr/bin/env python3
"""Frame indexes, proxies, tracking/pan and isolated provider job contract."""
import argparse
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / 'project'
    provider = root / 'provider'
    provider.write_text('''#!/usr/bin/env python3
import argparse,json,time
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--avw-request');p.add_argument('--avw-result');a=p.parse_args()
r=json.loads(Path(a.avw_request).read_text())
mode=r['parameters'].get('mode','good')
if mode=='sleep':
 Path(a.avw_result).with_name('provider-ready').touch()
 time.sleep(60)
output={'schemaVersion':1,'kind':'analysis' if mode!='bad' else 'edit','sourceSha256':r['source']['sha256'],'data':{'result':'fixture evidence'}}
Path(a.avw_result).write_text(json.dumps(output))
''')
    provider.chmod(0o755)
    ledger = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=120)
        assert result.returncode == 0, (argv, result.stdout, result.stderr)
        return result.stdout

    def call(command, **fields):
        request = {'command': command, **fields}
        response = json.loads(subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                              '--analysis-provider', str(provider), 'request', '-'],
                             input=json.dumps(request).encode(), capture_output=True, timeout=120).stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'], response
        return response['result']

    source = root / 'moving.mp4'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=0x404040:s=320x180:r=30:d=3',
         '-f', 'lavfi', '-i', 'color=c=white:s=48x48:r=30:d=3', '-filter_complex',
         "[1:v]drawbox=x=6:y=6:w=10:h=10:c=black:t=fill,drawbox=x=29:y=29:w=12:h=12:c=black:t=fill[p];[0:v][p]overlay=x='64+24*t':y=54",
         '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '14', source])
    call('create', project=str(project), name='Analysis suite')
    call('import', project=str(project), source=str(source), id='source', expectedRevision=0, key='import-source')

    def analyze(task, key):
        revision = call('status', project=str(project))['revision']
        return call('analyze-start', project=str(project), task=task, expectedRevision=revision,
                    key=key, noLaunch=True)

    def complete(job, expected='succeeded'):
        run([args.avw, 'worker', project])
        result = call('job-status', project=str(project), id=job['id'])
        assert result['state'] == expected, result
        if expected == 'succeeded':
            artifact = call('artifact', project=str(project), id=job['id'])
            return json.loads(Path(artifact['path']).read_text()), result
        return result

    base = {'assetId': 'source', 'startMs': 0, 'endMs': 3000}
    index, _ = complete(analyze({'kind': 'frame-index', **base}, 'source-frame-index'))
    assert len(index['data']['frames']) == 90, index
    proxy_task = {'kind': 'proxy', **base, 'width': 160, 'height': 90}
    proxy, job = complete(analyze(proxy_task, 'source-proxy'))
    assert proxy['data']['notForFinal'] and proxy['files']
    _, cached = complete(analyze(proxy_task, 'source-proxy-again'))
    assert cached['result']['cacheHit']
    track_task = {'kind': 'track', **base, 'region': {'x': .2, 'y': .3, 'width': .15, 'height': 48/180},
                  'sampleMs': 500, 'minConfidence': .6, 'maxMotionFraction': .2}
    tracking, _ = complete(analyze(track_task, 'track-selected-product'))
    proposal = tracking['data']
    assert len(proposal['points']) == 6
    errors = [abs(point['centerX'] * 320 - (88 + 24 * point['sourceMs']/1000))
              for point in proposal['points']]
    assert max(errors) <= 6, (errors, proposal)
    p = call('status', project=str(project))
    t = lambda value: {'value': value, 'rate': {'numerator': 30, 'denominator': 1}}
    call('apply', project=str(project), batch={'schemaVersion': '1.0.0', 'projectId': p['projectId'],
         'baseRevision': p['revision'], 'idempotencyKey': 'add-tracked-clip', 'operations': [
             {'id': 'clip', 'op': 'clip.add', 'params': {'id': 'clip', 'asset': 'source',
              'track': 'track_v1', 'at': t(0), 'sourceIn': t(0), 'duration': t(90), 'fit': 'cover'}}]})
    call('studio', project=str(project), edit={'action': 'reframe-apply', 'itemId': 'clip', 'proposal': proposal},
         expectedRevision=2, key='accept-tracking-proposal')
    render = call('render', project=str(project))
    centers = []
    for second in (.1, 1, 2):
        pixels = run([args.ffmpeg, '-v', 'error', '-ss', str(second), '-i', render['path'],
                      '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'gray', '-'])
        points = [i % 360 for i, value in enumerate(pixels) if value > 180]
        assert points
        centers.append(sum(points)/len(points))
    assert max(centers)-min(centers) < 12, centers
    call('studio', project=str(project), edit={'action': 'grade', 'itemId': 'clip',
         'effectId': 'brighten', 'brightness': .1, 'exposure': 0, 'contrast': 0, 'saturation': 1},
         expectedRevision=3, key='grade-tracked-clip')
    graded = call('render', project=str(project))
    def pixel(path):
        return run([args.ffmpeg, '-v', 'error', '-ss', '0.1', '-i', path,
                    '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'gray', '-'])[0]
    assert pixel(graded['path']) - pixel(render['path']) > 10
    # Occlusion must produce visible low-confidence findings and hold positions.
    occluded = root / 'occluded.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', source, '-vf',
         "drawbox=x=0:y=0:w=iw:h=ih:color=0x404040:t=fill:enable='gte(t,2)'",
         '-c:v', 'libx264', '-preset', 'ultrafast', occluded])
    call('import', project=str(project), source=str(occluded), id='occluded',
         expectedRevision=call('status', project=str(project))['revision'], key='import-occluded-source')
    hidden, _ = complete(analyze({**track_task, 'assetId': 'occluded'}, 'track-occluded-product'))
    assert hidden['data']['findings'] and all(p['held'] for p in hidden['data']['points'][-2:])
    # A modified proxy attachment is rejected on artifact retrieval.
    attachment = Path(proxy['files'][0]['name'])
    proxy_file = Path(job['result']['path']).parent / attachment
    original = proxy_file.read_bytes(); proxy_file.write_bytes(original + b'corrupt')
    request = {'command':'artifact','project':str(project),'id':job['id']}
    bad = subprocess.run([args.avw,'request','-'],input=json.dumps(request).encode(),capture_output=True)
    assert bad.returncode!=0 and not json.loads(bad.stdout)['ok']
    proxy_file.write_bytes(original)
    current = call('status', project=str(project))['revision']
    good = {'kind': 'provider', 'assetId': 'source', 'task': 'fixture', 'parameters': {}}
    complete(analyze(good, 'external-provider-good'))
    _, reused = complete(analyze(good, 'external-provider-reuse'))
    assert reused['result']['cacheHit']
    complete(analyze({**good, 'parameters': {'mode': 'bad'}}, 'external-provider-bad'), expected='failed')
    assert call('status', project=str(project))['revision'] == current
    cancelled = analyze({**good, 'parameters': {'mode': 'sleep'}}, 'external-provider-cancel')
    worker = subprocess.Popen([args.avw, 'worker', str(project)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    for _ in range(100):
        if list((project / 'cache').glob('analysis-*/provider-ready')):
            break
        time.sleep(.1)
    else:
        raise AssertionError('provider did not start')
    call('job-cancel', project=str(project), id=cancelled['id'])
    worker.communicate(timeout=5)
    assert call('job-status', project=str(project), id=cancelled['id'])['state'] == 'cancelled'
    assert call('status', project=str(project))['revision'] == current
    (root / 'summary.json').write_text(json.dumps({'passed': True, 'trackingCenterErrorsSourcePixels': errors,
        'renderedProductCenters': centers, 'checks': ['PTS-based frame index', 'mapped SDR proxy/cache reuse',
        'selected-region tracking with occlusion hold', 'proxy attachment tamper rejection', 'grade with pan', 'accepted pan renders motion', 'provider version/hash contract/cache',
        'malformed result and cancellation preserve history']}, indent=2))
    print('Analysis passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
