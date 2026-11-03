#!/usr/bin/env python3
"""Exercise exact subtitle sidecars and pre-encode refusal with generated media."""
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
    original = root / 'source.mp4'
    ledger = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        assert result.returncode == 0, (argv, result.stdout, result.stderr)
        return result.stdout

    def call(command, expect_ok=True, backend=None, **fields):
        request = {'command': command, 'project': str(project), **fields}
        ffmpeg, ffprobe = backend or (args.ffmpeg, args.ffprobe)
        result = subprocess.run([args.avw, '--ffmpeg', str(ffmpeg), '--ffprobe', str(ffprobe),
                                 'request', '-'], input=json.dumps(request).encode(),
                                capture_output=True, timeout=180)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == expect_ok and (result.returncode == 0) == expect_ok, response
        return response['result'] if expect_ok else response['error']

    def digest(path):
        return hashlib.sha256(Path(path).read_bytes()).hexdigest()

    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'color=c=red:s=320x180:r=30:d=2', '-f', 'lavfi', '-i',
         'sine=frequency=440:sample_rate=48000:duration=2', '-c:v', 'libx264',
         '-pix_fmt', 'yuv420p', '-color_primaries', 'bt709', '-color_trc', 'bt709',
         '-colorspace', 'bt709', '-color_range', 'tv', '-c:a', 'aac', '-shortest', original])
    source_hash = digest(original)
    call('create', name='Caption sidecar timing')
    call('import', source=str(original), id='source', expectedRevision=0, key='import-source')
    call('import', source=str(Path(args.font).resolve()), id='font', expectedRevision=1, key='import-font')
    call('transcript-import', expectedRevision=2, key='reviewed-cues', transcript={
        'assetId': 'source', 'language': 'en', 'provider': 'generated-fixture',
        'cues': [{'id': 'normal', 'startMs': 300, 'endMs': 1000, 'text': 'Readable caption'}]})
    call('compose', expectedRevision=3, key='compose-candidate', edit={
        'outputId': 'candidate', 'name': 'Captioned cut', 'fontAssetId': 'font',
        'width': 360, 'height': 640, 'fontSize': 20,
        'cuts': [{'id': 'take', 'assetId': 'source', 'startMs': 0, 'endMs': 1500}]})
    earlier = call('render', sequence='candidate')
    prior_files = [Path(earlier['path']), Path(earlier['manifest'])]
    prior_files.extend(Path(earlier['path']).parent / name for name in ('captions.srt', 'captions.vtt'))
    prior_hashes = {str(path): digest(path) for path in prior_files}
    assert call('artifact', id=earlier['artifactId'])['verified']
    for extension, other_format in [('srt', 'webvtt'), ('vtt', 'srt')]:
        sidecar = Path(earlier['path']).parent / ('captions.' + extension)
        packet_data = json.loads(run([args.ffprobe, '-v', 'error', '-show_entries',
                                      'packet=pts_time,duration_time', '-of', 'json', sidecar]))
        assert len(packet_data['packets']) == 1, packet_data
        packet = packet_data['packets'][0]
        assert abs(float(packet['pts_time']) - 0.3) < 0.001, packet
        assert abs(float(packet['duration_time']) - 0.7) < 0.001, packet
        decoded = run([args.ffmpeg, '-v', 'error', '-i', sidecar,
                       '-f', other_format, 'pipe:1']).decode()
        assert 'Readable caption' in decoded, decoded

    # Check the actual queued deliverable, not only an in-memory probe fixture.
    queued_normal = call('render-start', sequence='candidate', expectedRevision=4,
                         key='queued-normal-captions', noLaunch=True)
    run([args.avw, 'worker', project])
    completed = call('job-status', id=queued_normal['id'])
    assert completed['state'] == 'succeeded', completed
    report = call('delivery-check', id=queued_normal['id'], brief={
        'schemaVersion': 1, 'requiredSidecars': ['srt', 'vtt']})
    assert report['status'] == 'pass' and all(check['status'] == 'pass'
                                              for check in report['checks']), report
    queued_path = Path(completed['result']['path'])
    queued_files = [queued_path, Path(completed['result']['manifest'])]
    queued_files.extend(queued_path.parent / name for name in ('captions.srt', 'captions.vtt'))
    queued_hashes = {str(path): digest(path) for path in queued_files}
    sidecar = queued_path.parent / 'captions.srt'
    original_sidecar = sidecar.read_bytes()
    try:
        sidecar.write_bytes(b'altered sidecar')
        damaged = call('delivery-check', id=queued_normal['id'], brief={
            'schemaVersion': 1, 'requiredSidecars': ['srt', 'vtt']})
        assert damaged['status'] == 'fail', damaged
        assert any(check['requested'] == 'srt' and check['status'] == 'fail'
                   for check in damaged['checks']), damaged
    finally:
        sidecar.write_bytes(original_sidecar)

    status = call('status')
    source_asset = next(asset for asset in status['assets'] if asset['id'] == 'source')
    imported_original = project / source_asset['uri']
    assert digest(imported_original) == source_hash
    small_rate = {'numerator': 10000, 'denominator': 1}
    call('apply', batch={'schemaVersion': '1.0.0', 'projectId': status['projectId'],
         'baseRevision': status['revision'], 'idempotencyKey': 'add-short-cue', 'operations': [
             {'id': 'short-track', 'op': 'track.add', 'params': {
                 'id': 'short_track', 'sequence': 'candidate', 'type': 'caption'}},
             {'id': 'short-caption', 'op': 'caption.add', 'params': {
                 'id': 'collapsed_cue', 'track': 'short_track',
                 'at': {'value': 1, 'rate': small_rate},
                 'duration': {'value': 3, 'rate': small_rate}, 'text': 'Brief'}}]})
    revision = call('status')['revision']
    renders = project / 'renders'
    before = sorted(str(path.relative_to(renders)) for path in renders.rglob('*'))
    # An invocation would create this marker. Refusal before either tool runs
    # demonstrates the invalid cue is caught before probing or encoding.
    marker = root / 'backend-invoked'
    sentinel = root / 'sentinel-backend'
    sentinel.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\n'
                        f'Path({str(marker)!r}).write_text("invoked")\nraise SystemExit(1)\n')
    sentinel.chmod(0o755)
    error = call('render', expect_ok=False, sequence='candidate', backend=(sentinel, sentinel))
    assert error['code'] == 'E_INVALID_REQUEST', error
    assert 'collapsed_cue' in error['message'] and 'millisecond rounding' in error['message'], error
    assert not marker.exists(), 'backend ran before cue refusal'

    queued = call('render-start', sequence='candidate', expectedRevision=revision,
                  key='queued-short-cue', noLaunch=True, backend=(sentinel, sentinel))
    run([args.avw, 'worker', project])
    failed = call('job-status', id=queued['id'])
    assert failed['state'] == 'failed' and failed['result'] is None, failed
    assert 'collapsed_cue' in failed['error'] and 'millisecond rounding' in failed['error'], failed
    assert not marker.exists(), 'queued worker ran backend before cue refusal'
    assert sorted(str(path.relative_to(renders)) for path in renders.rglob('*')) == before
    assert call('status')['revision'] == revision
    assert digest(original) == source_hash
    assert digest(imported_original) == source_hash
    assert {str(path): digest(path) for path in prior_files} == prior_hashes
    assert {str(path): digest(path) for path in queued_files} == queued_hashes
    assert call('artifact', id=earlier['artifactId'])['verified']
    (root / 'summary.json').write_text(json.dumps({
        'passed': True, 'collapsedCaptionId': 'collapsed_cue',
        'syncAndQueuedPreflightRanBeforeBackend': True, 'newArtifactNotPublished': True,
        'priorVideoManifestAndSidecarsUnchanged': True, 'sourceUnchanged': True,
        'importedOriginalUnchanged': True,
        'srtAndVttParsedWithFfprobe': True, 'srtAndVttTextDecodedWithFfmpeg': True,
        'queuedCaptionSidecarsPassDeliveryCheck': True, 'damagedSidecarFails': True,
        'normalCueStartSeconds': 0.3, 'normalCueDurationSeconds': 0.7,
    }, indent=2) + '\n')
    print('Caption sidecar timing passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
