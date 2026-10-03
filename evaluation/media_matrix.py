#!/usr/bin/env python3
"""Synthetic SDR/HDR input qualification with decoded reference comparisons."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import array


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
         '-pix_fmt', 'yuv420p', '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709',
         '-colorspace', 'bt709', base])
    accepted = []
    for rotation in (0, 90, 180, 270):
        source = root / f'rotation-{rotation}.mov'
        run([args.ffmpeg, '-v', 'error', '-display_rotation', rotation, '-i', base,
             '-c', 'copy', source])
        accepted.append((f'rotation_{rotation}', source))
    sar = root / 'sar.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-vf', 'setsar=2',
         '-c:v', 'libx264', '-preset', 'ultrafast', sar])
    accepted.append(('sar', sar))
    sar_rotated = root / 'sar-rotated.mov'
    run([args.ffmpeg, '-v', 'error', '-display_rotation', '90', '-i', sar,
         '-c', 'copy', sar_rotated])
    accepted.append(('sar_rotated', sar_rotated))
    hevc = root / 'hevc.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-c:v', 'libx265',
         '-x265-params', 'pools=2:frame-threads=2:log-level=error', '-tag:v', 'hvc1', hevc])
    accepted.append(('hevc_sdr', hevc))
    vfr = root / 'vfr.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-vf',
         "select='if(lt(t,1.5),not(mod(n,2)),1)'", '-fps_mode', 'vfr',
         '-c:v', 'libx264', '-preset', 'ultrafast', vfr])
    accepted.append(('vfr', vfr))
    hdr_filter = 'zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=mobius:param=0.3:desat=2:peak=10,zscale=t=bt709:m=bt709:r=limited:dither=error_diffusion,format=yuv420p'
    for name, transfer in [('pq', 'smpte2084'), ('hlg', 'arib-std-b67')]:
        source = root / f'{name}.mp4'
        run([args.ffmpeg, '-v', 'error', '-i', base, '-vf',
             f'zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt2020:t={transfer}:m=bt2020nc:r=limited,format=yuv420p10le',
             '-c:v', 'libx265', '-x265-params', f'pools=2:frame-threads=2:log-level=error:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc', source])
        accepted.append((name, source))
    offset = root / 'offset.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-c', 'copy', '-output_ts_offset', '2', offset])
    accepted.append(('offset', offset))
    delayed = root / 'delayed-audio.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', base, '-itsoffset', '1', '-f', 'lavfi',
         '-i', 'sine=frequency=440:sample_rate=48000:duration=2', '-map', '0:v', '-map', '1:a',
         '-c:v', 'copy', '-c:a', 'aac', delayed])
    accepted.append(('delayed_audio', delayed))
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
        if name.startswith('rotation_') or name in ('pq', 'hlg', 'offset', 'sar', 'sar_rotated'):
            def pixels(path, reference=False):
                argv = [args.ffmpeg, '-v', 'error', *(['-noautorotate'] if reference else []), '-ss', '1', '-i', path]
                if reference:
                    filters = []
                    if name in ('sar', 'sar_rotated'):
                        filters += ['scale=iw*sar:ih', 'setsar=1']
                    rotation = 90 if name == 'sar_rotated' else (int(name.removeprefix('rotation_')) if name.startswith('rotation_') else 0)
                    if rotation:
                        filters += [{90: 'transpose=2', 180: 'hflip,vflip', 270: 'transpose=1'}[rotation]]
                    if name in ('pq', 'hlg'):
                        filters += [hdr_filter]
                    filters += ['scale=360:640:force_original_aspect_ratio=increase', 'crop=360:640', 'setsar=1', 'format=gbrp', 'scale=out_color_matrix=bt709:out_range=tv', 'format=yuv420p']
                    argv += ['-vf', ','.join(filters)]
                return run([*argv, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])
            reference, actual = pixels(source, True), pixels(artifact['path'])
            assert len(reference) == len(actual) == 360 * 640 * 3
            difference = sum(abs(a - b) for a, b in zip(reference, actual)) / len(actual)
            assert difference < 8, (name, difference)
        audio_gap = None
        if name == 'delayed_audio':
            pcm = array.array('h', run([args.ffmpeg, '-v', 'error', '-i', artifact['path'],
                    '-vn', '-ac', '1', '-ar', '48000', '-f', 's16le', '-']))
            def amplitude(at):
                segment = pcm[int(at*48000):int((at+.1)*48000)]
                return (sum(v*v for v in segment)/len(segment))**.5
            assert amplitude(.1)<10 and amplitude(1.2)>1000, (amplitude(.1), amplitude(1.2))
            audio_gap = {'initialSilenceRms':amplitude(.1), 'laterToneRms':amplitude(1.2)}
        results.append({'fixture': name, 'decodedFrames': 60,
                        'sourceUnchanged': True, 'orientationMeanAbsoluteDifference': difference, 'audioGap':audio_gap})
    summary = {'passed': True, 'accepted': results,
               'limitations': ['Generated inputs; real camera appearance/audio needs human review',
                               'Dolby Vision profile 5 is inspectable but has no delivery path',
                               'VFR fixture verifies output cadence/count, not lip sync in real speech']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print('Media matrix passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
