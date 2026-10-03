#!/usr/bin/env python3
"""Synthetic SDR input qualification and precise rejection regressions."""
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
    ledger = []

    def run(argv, expected=0):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=120)
        assert result.returncode == expected, (argv, result.stderr, result.stdout)
        return result.stdout

    def avw(*argv, expected=0):
        command = [args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe, *argv]
        raw = run(command, expected)
        response = json.loads(raw)
        ledger.append({'args': list(map(str, argv)), 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == (expected == 0), response
        return response.get('result', response)

    base = root / 'base.mp4'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'testsrc2=s=640x360:r=30:d=3', '-c:v', 'libx264', '-preset', 'ultrafast',
         '-pix_fmt', 'yuv420p', '-color_primaries', 'bt709', '-color_trc', 'bt709',
         '-colorspace', 'bt709', base])
    accepted = []
    for rotation in (0, 90, 180, 270):
        source = root / f'rotation-{rotation}.mov'
        run([args.ffmpeg, '-v', 'error', '-display_rotation', rotation, '-i', base,
             '-c', 'copy', source])
        accepted.append((f'rotation_{rotation}', source))
    hevc = root / 'hevc.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-c:v', 'libx265',
         '-x265-params', 'pools=2:frame-threads=2:log-level=error', '-tag:v', 'hvc1', hevc])
    accepted.append(('hevc_sdr', hevc))
    vfr = root / 'vfr.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-vf',
         "select='if(lt(t,1.5),not(mod(n,2)),1)'", '-fps_mode', 'vfr',
         '-c:v', 'libx264', '-preset', 'ultrafast', vfr])
    accepted.append(('vfr', vfr))
    results = []
    for name, source in accepted:
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        project = root / name
        avw('create', project, '--name', name)
        avw('import', project, source, '--id', 'source', '--expected-revision', 0,
            '--key', 'import-source')
        state = avw('status', project)
        batch = {'schemaVersion': '1.0.0', 'projectId': state['projectId'],
                 'baseRevision': 1, 'idempotencyKey': 'add-video', 'operations': [{
                     'id': 'video', 'op': 'clip.add', 'params': {
                         'id': 'clip', 'asset': 'source', 'track': 'track_v1',
                         'at': {'value': 0, 'rate': {'numerator': 30, 'denominator': 1}},
                         'sourceIn': {'value': 0, 'rate': {'numerator': 30, 'denominator': 1}},
                         'duration': {'value': 60, 'rate': {'numerator': 30, 'denominator': 1}},
                         'fit': 'cover'}}]}
        request = root / f'{name}.json'
        request.write_text(json.dumps(batch))
        avw('apply', project, '--request', request)
        artifact = avw('render', project)
        manifest = json.loads(Path(artifact['manifest']).read_text())
        assert manifest['verification']['expectedFrames'] == 60
        assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
        difference = None
        if name.startswith('rotation_'):
            def pixels(path, reference=False):
                argv = [args.ffmpeg, '-v', 'error', '-ss', '1', '-i', path]
                if reference:
                    argv += ['-vf', 'scale=360:640:force_original_aspect_ratio=increase,crop=360:640,setsar=1']
                return run([*argv, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])
            reference, actual = pixels(source, True), pixels(artifact['path'])
            assert len(reference) == len(actual) == 360 * 640 * 3
            difference = sum(abs(a - b) for a, b in zip(reference, actual)) / len(actual)
            assert difference < 8, (name, difference)
        results.append({'fixture': name, 'decodedFrames': 60,
                        'sourceUnchanged': True, 'orientationMeanAbsoluteDifference': difference})
    rejected = []
    for name, options, message in [
        ('pq', ['-x264-params', 'colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc'], 'HDR'),
        ('hlg', ['-x264-params', 'colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc'], 'HDR'),
        ('offset', ['-output_ts_offset', '2'], 'nonzero stream start'),
    ]:
        source = root / f'{name}.mp4'
        run([args.ffmpeg, '-v', 'error', '-i', base, '-c:v', 'libx264',
             '-preset', 'ultrafast', *options, source])
        if name in ('pq', 'hlg'):
            probe = json.loads(run([args.ffprobe, '-v', 'error', '-show_streams', '-of', 'json', source]))
            assert probe['streams'][0]['color_transfer'] == {'pq':'smpte2084', 'hlg':'arib-std-b67'}[name], probe
        project = root / f'reject_{name}'
        avw('create', project, '--name', name)
        response = avw('import', project, source, '--id', 'source',
                       '--expected-revision', 0, '--key', 'reject-source', expected=1)
        assert message in response['error']['message'], response
        assert avw('status', project)['revision'] == 0
        assert not list((project / 'originals').iterdir())
        assert avw('imports', project)[0]['state'] == 'failed'
        rejected.append(name)
    summary = {'passed': True, 'accepted': results, 'rejected': rejected,
               'limitations': ['Generated inputs only; real camera color/audio and Dolby Vision remain unqualified',
                               'VFR fixture verifies output cadence/count, not lip sync in real speech']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print('Media matrix passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
