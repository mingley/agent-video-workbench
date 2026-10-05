#!/usr/bin/env python3
"""Generated Rec.709 color patches and a full-HD multi-cut HLG memory regression."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'font', 'output'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--color-only', action='store_true')
    a = parser.parse_args()
    root = Path(a.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / 'project'
    ledger = []

    def run(argv):
        p = subprocess.run(list(map(str, argv)), capture_output=True, timeout=300)
        assert p.returncode == 0, (argv, p.stderr.decode(errors='replace'))
        return p.stdout

    def call(command, **fields):
        request = {'command': command, 'project': str(project), **fields}
        p = subprocess.run([a.avw, '--ffmpeg', a.ffmpeg, '--ffprobe', a.ffprobe,
                            'request', '-'], input=json.dumps(request).encode(),
                           capture_output=True, timeout=300)
        response = json.loads(p.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert p.returncode == 0 and response['ok'], response
        return response['result']

    def mutate(command, key, **fields):
        return call(command, expectedRevision=call('status')['revision'], key=key, **fields)

    # Encode known RGB patches with explicit Rec.709 matrix/range. Interior
    # patches avoid chroma edges and compare decoded source vs delivered pixels.
    colors = [(230, 20, 30), (20, 220, 40), (10, 50, 235), (240, 190, 10),
              (180, 110, 80), (225, 165, 130), (100, 100, 100), (235, 235, 235)]
    ppm = root / 'patches.ppm'
    ppm.write_bytes(b'P6\n640 360\n255\n' + bytes(
        component for y in range(360) for x in range(640)
        for component in colors[(y // 180) * 4 + x // 160]))
    source = root / 'patches.mp4'
    run([a.ffmpeg, '-v', 'error', '-loop', '1', '-framerate', '30', '-i', ppm,
         '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=2',
         '-t', '2', '-vf', 'scale=out_color_matrix=bt709:out_range=tv,format=yuv420p',
         '-c:v', 'libx264', '-threads', '2', '-crf', '0', '-preset', 'ultrafast',
         '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
         '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709',
         '-c:a', 'aac', source])
    hdr_patches = root / 'hdr-patches.mp4'
    hdr_encode = ('zscale=t=linear:npl=100,format=gbrpf32le,'
                  'zscale=p=bt2020:t=arib-std-b67:m=bt2020nc:r=limited,'
                  'format=yuv420p10le')
    run([a.ffmpeg, '-v', 'error', '-threads', '2', '-i', source, '-vf', hdr_encode,
         '-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params',
         'pools=2:frame-threads=2:lossless=1:log-level=error', '-c:a', 'copy', hdr_patches])
    call('create', name='Color and decoder budget regression')
    mutate('import', 'import-patches', source=str(hdr_patches), id='patches')
    mutate('import', 'import-font', source=a.font, id='font')
    mutate('transcript-import', 'patch-cue', transcript={'assetId': 'patches',
           'language': 'en', 'provider': 'generated fixture', 'cues': [
               {'id': 'cue', 'startMs': 0, 'endMs': 1800, 'text': 'RGB compositing'}]})
    mutate('compose', 'compose-patches', edit={'outputId': 'color', 'name': 'Color',
           'fontAssetId': 'font', 'width': 640, 'height': 360, 'fontSize': 24,
           'cuts': [{'id': 'cut', 'assetId': 'patches', 'startMs': 0, 'endMs': 1800}]})
    result = call('render', sequence='color')

    def pixels(path, tone_map=False):
        args = [a.ffmpeg, '-v', 'error', '-threads', '2', '-i', path]
        if tone_map:
            # Hold the existing SDR policy constant. This reference bypasses
            # compositing, and does not certify the HDR tone-map appearance.
            args += ['-vf', 'zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,'
                     'tonemap=tonemap=mobius:param=0.3:desat=2:peak=10,'
                     'zscale=t=bt709:m=bt709:r=limited:dither=error_diffusion,format=yuv420p']
        return run([*args, '-frames:v', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'])

    before, after = pixels(hdr_patches, tone_map=True), pixels(result['path'])
    errors = []
    for y in (80, 230):
        for x in (80, 240, 400, 560):
            for c in range(3):
                offsets = [(yy * 640 + xx) * 3 + c
                           for yy in range(y - 8, y + 8) for xx in range(x - 8, x + 8)]
                errors.append(abs(sum(before[i] for i in offsets) -
                                  sum(after[i] for i in offsets)) / len(offsets))
    color_error = max(errors)
    (root / 'color-error.json').write_text(json.dumps({'maxPatchChannelError': color_error}))
    # Permit three 8-bit levels for the final lossy encode/rounding, but not
    # the extra matrix conversion introduced by YUV compositing.
    assert color_error <= 3, ('unintended compositing color shift', color_error)
    summary = {'passed': True, 'maxPatchChannelError': color_error,
               'colorTolerance8Bit': 3, 'hdrPreservationQualified': False}
    if not a.color_only:
        hdr = root / 'hlg.mp4'
        run([a.ffmpeg, '-v', 'error', '-stream_loop', '-1', '-threads', '2', '-i', source,
             '-t', '30', '-vf', 'scale=1080:1920,zscale=t=linear:npl=100,'
             'format=gbrpf32le,zscale=p=bt2020:t=arib-std-b67:m=bt2020nc:r=limited,'
             'format=yuv420p10le', '-c:v', 'libx265', '-preset', 'ultrafast',
             '-x265-params', 'pools=2:frame-threads=2:log-level=error',
             '-color_primaries', 'bt2020', '-color_trc', 'arib-std-b67',
             '-colorspace', 'bt2020nc', '-c:a', 'aac', hdr])
        digest = hashlib.sha256(hdr.read_bytes()).hexdigest()
        mutate('import', 'import-hlg', source=str(hdr), id='hlg')
        mutate('transcript-import', 'hdr-cue', transcript={'assetId': 'hlg',
               'language': 'en', 'provider': 'generated fixture', 'cues': [
                   {'id': 'cue', 'startMs': 0, 'endMs': 29000, 'text': 'Full HD HLG'}]})
        cuts = [(16900, 22800), (6100, 7500), (13800, 16000), (26300, 29900)]
        mutate('compose', 'compose-hlg', edit={'outputId': 'hdr', 'name': 'HDR cuts',
               'fontAssetId': 'font', 'cuts': [
                   {'id': 'c' + str(i), 'assetId': 'hlg', 'startMs': start, 'endMs': end}
                   for i, (start, end) in enumerate(cuts)]})
        started = time.monotonic()
        result = call('render', sequence='hdr')
        manifest = json.loads(Path(result['manifest']).read_text())
        assert manifest['verification']['decoded']
        assert manifest['verification']['expectedFrames'] == 393
        assert manifest['resourcePolicy']['childAddressSpaceBytes'] == 4294967296
        assert manifest['resourcePolicy']['decoderThreadsPerInput'] == 1
        assert sorted(manifest['inputSeeksSeconds'].values()) == [6, 13, 16, 26]
        assert all(d['assetId'] == 'hlg' and d['sourceSha256'] == digest
                   for d in manifest['colorDecisions'])
        assert len(manifest['colorDecisions']) == 4
        assert {asset['id'] for asset in call('status')['assets']} == {'patches', 'font', 'hlg'}
        assert hashlib.sha256(hdr.read_bytes()).hexdigest() == digest
        summary.update({'fullHdHlgFrames': 393, 'renderSeconds': time.monotonic() - started,
                        'workerAddressSpaceBytes': 4294967296, 'sourceUnchanged': True})
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
