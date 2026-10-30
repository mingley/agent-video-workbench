#!/usr/bin/env python3
"""Generated-media check for captions that round below one output frame."""
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

    def call(command, expect_ok=True, **fields):
        request = {'command': command, 'project': str(project), **fields}
        result = subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                                 'request', '-'], input=json.dumps(request).encode(),
                                capture_output=True, timeout=180)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == expect_ok and (result.returncode == 0) == expect_ok, response
        return response['result'] if expect_ok else response['error']

    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'color=c=red:s=320x180:r=30:d=2', '-f', 'lavfi', '-i',
         'sine=frequency=440:sample_rate=48000:duration=2', '-c:v', 'libx264',
         '-pix_fmt', 'yuv420p', '-color_primaries', 'bt709', '-color_trc', 'bt709',
         '-colorspace', 'bt709', '-color_range', 'tv', '-c:a', 'aac', '-shortest', original])
    original_sha = hashlib.sha256(original.read_bytes()).hexdigest()
    call('create', name='Caption frame timing')
    call('import', source=str(original), id='source', expectedRevision=0, key='import-source')
    call('import', source=str(Path(args.font).resolve()), id='font', expectedRevision=1, key='import-font')
    call('transcript-import', expectedRevision=2, key='reviewed-cues', transcript={
        'assetId': 'source', 'language': 'en', 'provider': 'generated-fixture',
        'cues': [{'id': 'brief', 'startMs': 200, 'endMs': 220, 'text': 'Brief'},
                 {'id': 'boundary', 'startMs': 990, 'endMs': 1020, 'text': 'Words at boundary'}]})
    edit = {'outputId': 'candidate', 'name': 'Captioned cut', 'fontAssetId': 'font',
            'width': 360, 'height': 640, 'fontSize': 20,
            'cuts': [{'id': 'take', 'assetId': 'source', 'startMs': 0, 'endMs': 1000}]}
    before = call('status')
    history = call('history')
    error = call('compose', expect_ok=False, edit=edit, expectedRevision=3, key='compose-candidate')
    assert error['code'] == 'E_INVALID_REQUEST'
    assert all(part in error['message'] for part in ('boundary', 'source', 'take', '990..1000ms',
                                                       'zero output frames')), error
    assert call('status') == before and call('history') == history
    assert call('request-outcome', key='compose-candidate')['outcome'] is None

    # Extending the cut makes both words visible; the failed key is reusable.
    edit['cuts'][0]['endMs'] = 1050
    call('compose', edit=edit, expectedRevision=3, key='compose-candidate')
    result = call('render', sequence='candidate')
    artifact = call('artifact', id=result['artifactId'])
    assert artifact['verified'] and artifact['sha256'] == hashlib.sha256(Path(result['path']).read_bytes()).hexdigest()
    manifest = json.loads(Path(result['manifest']).read_text())
    assert manifest['verification']['expectedFrames'] == 32
    srt = (Path(result['path']).parent / 'captions.srt').read_text()
    vtt = (Path(result['path']).parent / 'captions.vtt').read_text()
    for text in ('Brief', 'Words at boundary'):
        assert text in srt and text in vtt
    assert '00:00:00,200 --> 00:00:00,233' in srt
    assert '00:00:01,000 --> 00:00:01,033' in srt

    # The one-frame cues must also appear in the burned video, not just SRT.
    pixels = run([args.ffmpeg, '-v', 'error', '-i', result['path'], '-vf',
                  r'select=eq(n\,5)+eq(n\,6)+eq(n\,29)+eq(n\,30),crop=360:250:0:390',
                  '-fps_mode', 'passthrough', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'])
    frame_bytes = 360 * 250 * 3
    assert len(pixels) == 4 * frame_bytes
    def bright(index):
        frame = pixels[index * frame_bytes:(index + 1) * frame_bytes]
        return sum(1 for p in range(0, len(frame), 3)
                   if frame[p] > 200 and frame[p + 1] > 180 and frame[p + 2] > 180)
    white_pixels = [bright(index) for index in range(4)]
    assert white_pixels[1] > white_pixels[0] + 10, white_pixels
    assert white_pixels[3] > white_pixels[2] + 10, white_pixels
    assert hashlib.sha256(original.read_bytes()).hexdigest() == original_sha
    (root / 'summary.json').write_text(json.dumps({
        'passed': True, 'collapsedCueRejectedAtomically': True, 'failedKeyReusable': True,
        'cueIntervalsMs': [[200, 220], [990, 1020]], 'verifiedRenderFrames': 32,
        'sidecarsRetainBothCues': True, 'captionFrameWhitePixels': white_pixels,
        'sourceUnchanged': True,
    }, indent=2) + '\n')
    print('Caption frame timing passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
