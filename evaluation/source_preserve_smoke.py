#!/usr/bin/env python3
"""Generated 10-bit PQ/HLG preservation, timing, preview and refusal regression.

Pixel equality is evaluated in decoded native YUV, independent of an HDR screen.
This does not qualify proprietary Dolby Vision metadata or artistic appearance.
"""
import argparse
from array import array
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--large-source', help='Generated full-HD HLG fixture from color_memory_smoke.py')
    a = parser.parse_args()
    root = Path(a.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / 'project'
    ledger = []

    def run(argv):
        p = subprocess.run(list(map(str, argv)), capture_output=True, timeout=300)
        assert p.returncode == 0, (argv, p.stderr.decode(errors='replace'))
        return p.stdout

    def call(command, expect_ok=True, **fields):
        request = {'command': command, 'project': str(project), **fields}
        p = subprocess.run([a.avw, '--ffmpeg', a.ffmpeg, '--ffprobe', a.ffprobe,
                            'request', '-'], input=json.dumps(request).encode(),
                           capture_output=True, timeout=300)
        response = json.loads(p.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == expect_ok, response
        assert (p.returncode == 0) == expect_ok, response
        return response['result'] if expect_ok else response['error']

    def mutate(command, key, **fields):
        return call(command, expectedRevision=call('status')['revision'], key=key, **fields)

    def raw(path, seconds=None, frames=None):
        args = [a.ffmpeg, '-v', 'error', '-threads', '1', '-i', path]
        if seconds is not None:
            args += ['-ss', str(seconds)]
        if frames is not None:
            args += ['-frames:v', str(frames)]
        return run([*args, '-an', '-pix_fmt', 'yuv420p10le', '-f', 'rawvideo', '-'])

    def probe(path, initial=False):
        extra = ['-read_intervals', '%+#1', '-show_frames'] if initial else ['-count_frames']
        return json.loads(run([a.ffprobe, '-v', 'error', '-threads', '1', *extra,
                               '-show_streams', '-show_format', '-of', 'json', path]))

    # Many more than 256 distinct Y values, with non-neutral chroma. Testing
    # tagged sources through an 8-bit working space would fail byte equality.
    w, h, frame_bytes = 640, 360, 640 * 360 * 3
    samples = array('H', (64 + x for y in range(h) for x in range(w)))
    for plane in (0, 1):
        samples.extend(412 + ((x + plane * 80) % 200)
                       for y in range(h // 2) for x in range(w // 2))
    if sys.byteorder != 'little':
        samples.byteswap()
    seed = root / 'ramp.yuv'
    bright = array('H', [723]) * (w * h)
    bright.extend([512] * (w * h // 2))
    if sys.byteorder != 'little':
        bright.byteswap()
    # Author 10-bit light bursts directly: drawing an 8-bit overlay in the
    # fixture would itself reduce the source precision before the tool runs.
    with seed.open('wb') as file:
        for frame in range(180):
            file.write((bright if 60 <= frame < 66 or 135 <= frame < 141 else samples).tobytes())
    call('create', name='Native source preservation qualification')
    checks = []
    for label, transfer in (('pq', 'smpte2084'), ('hlg', 'arib-std-b67')):
        source = root / (label + '.mp4')
        params = ('pools=2:frame-threads=1:lossless=1:log-level=error:'
                  'colorprim=bt2020:colormatrix=bt2020nc:range=limited:transfer=' + transfer)
        if label == 'pq':
            params += (':master-display=G(13250,34500)B(7500,3000)R(34000,16000)'
                       'WP(15635,16450)L(10000000,50):max-cll=1000,400')
        run([a.ffmpeg, '-v', 'error', '-f', 'rawvideo',
             '-pixel_format', 'yuv420p10le', '-video_size', '640x360', '-framerate', '60',
             '-color_range', 'tv', '-color_primaries', 'bt2020', '-color_trc', transfer,
             '-colorspace', 'bt2020nc', '-i', seed, '-f', 'lavfi', '-i',
             "aevalsrc='0.5*sin(2*PI*1000*t)*(between(t,1,1.1)+between(t,2.25,2.35))':s=48000:d=3",
             '-t', '3',
             '-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', params,
             '-pix_fmt', 'yuv420p10le', '-color_primaries', 'bt2020', '-color_trc', transfer,
             '-colorspace', 'bt2020nc', '-color_range', 'tv', '-tag:v', 'hvc1',
             '-c:a', 'aac', '-b:a', '256k', source])
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        mutate('import', 'import-' + label, source=str(source), id=label)
        edit = {'outputId': label + '-native', 'name': label + ' natural cuts', 'quality': 'lossless',
                'cuts': [{'id': 'first', 'assetId': label, 'startMs': 500, 'endMs': 1500},
                         {'id': 'second', 'assetId': label, 'startMs': 2000, 'endMs': 2500}]}
        mutate('assemble', 'assemble-' + label, edit=edit)
        report = call('edit-preflight', sequence=label + '-native')
        assert report['outputColor']['transfer'] == transfer
        assert report['canvas']['width'] == w and report['frameRate']['numerator'] == 60
        assert not report['automaticCaptions'] and not report['automaticCrop']
        revision = call('status')['revision']
        result = call('render', sequence=label + '-native')
        manifest = json.loads(Path(result['manifest']).read_text())
        output = Path(result['path'])
        reference = raw(source, 0.5, 60) + raw(source, 2, 30)
        delivered = raw(output)
        assert len(delivered) == frame_bytes * 90
        assert delivered == reference, (label, 'native 10-bit pixels changed')
        unique_y = len(set(array('H', delivered[:w * h * 2])))
        assert unique_y >= 600, unique_y
        flash_starts = []
        prior_bright = False
        for frame in range(90):
            luma = array('H', delivered[frame * frame_bytes:frame * frame_bytes + w * h * 2])
            if sys.byteorder != 'little':
                luma.byteswap()
            on = sum(luma) / len(luma) > 650
            if on and not prior_bright:
                flash_starts.append(frame / 60)
            prior_bright = on
        assert flash_starts == [0.5, 1.25], flash_starts
        video = next(s for s in probe(output)['streams'] if s['codec_type'] == 'video')
        assert video['codec_name'] == 'hevc' and video['pix_fmt'] == 'yuv420p10le'
        assert video['avg_frame_rate'] == '60/1' and video['nb_read_frames'] == '90'
        assert video['color_transfer'] == transfer and video['color_range'] == 'tv'
        assert video['color_primaries'] == 'bt2020' and video['color_space'] == 'bt2020nc'
        source_probe, output_probe = probe(source, True), probe(output, True)
        static = lambda p: {json.dumps(d, sort_keys=True) for item in p.get('streams', []) + p.get('frames', [])
                            for d in item.get('side_data_list', []) if d['side_data_type']
                            in ('Mastering display metadata', 'Content light level metadata')}
        assert static(source_probe) == static(output_probe), (static(source_probe), static(output_probe))
        assert (label != 'pq') or len(static(output_probe)) == 2, 'HDR10 metadata missing in fixture'
        audio = array('f', run([a.ffmpeg, '-v', 'error', '-threads', '1', '-i', output,
                               '-vn', '-ac', '1', '-ar', '48000', '-f', 'f32le', '-']))
        if sys.byteorder != 'little':
            audio.byteswap()
        starts = []
        active = False
        for i in range(0, len(audio), 480):
            values = audio[i:i + 480]
            on = sum(v * v for v in values) / len(values) > 0.02
            if on and not active:
                starts.append(i / 48000)
            active = on
        assert len(starts) == 2 and all(abs(x - y) <= 0.02
                                       for x, y in zip(starts, (0.5, 1.25))), starts
        preview = output.parent / 'review-sdr.mp4'
        pv = next(s for s in probe(preview)['streams'] if s['codec_type'] == 'video')
        assert pv['codec_name'] == 'h264' and pv['pix_fmt'] == 'yuv420p'
        assert pv['color_transfer'] == 'bt709' and pv['nb_read_frames'] == '90'
        assert manifest['delivery']['reviewVideo'] == preview.name
        for entry in manifest['delivery']['files']:
            assert hashlib.sha256((output.parent / entry['path']).read_bytes()).hexdigest() == entry['sha256']
        package = root / (label + '-delivery')
        call('delivery', id=result['artifactId'], destination=str(package))
        html = (package / 'index.html').read_text()
        assert '/review-sdr.mp4' in html and 'Master' in html
        package = package / result['artifactId']
        assert (package / 'review-sdr.mp4').read_bytes() == preview.read_bytes()
        assert (package / 'video.mp4').read_bytes() == output.read_bytes()
        assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
        assert call('status')['revision'] == revision
        plan = json.loads((output.parent / 'plan.json').read_text())
        graph = plan['args'][plan['args'].index('-filter_complex') + 1]
        assert 'gbrp' not in graph and 'tonemap' not in graph and 'overlay' not in graph
        assert manifest['editPreflight']['masterVideoEncodingGenerations'] == 1
        checks.append({'transfer': transfer, 'nativePixelEquality': True, 'uniqueLumaValues': unique_y,
                       'frames': 90, 'lightBurstStarts': flash_starts, 'audioBurstStarts': starts, 'staticMetadataPreserved': True,
                       'verifiedSdrPreview': True, 'sourceUnchanged': True, 'revision': revision,
                       'artifactId': result['artifactId'], 'manifest': result['manifest']})
        if label == 'pq':
            edit['outputId'], edit['quality'] = 'pq-high', 'high'
            mutate('assemble', 'assemble-high', edit=edit)
            high = call('render', sequence='pq-high')
            before, after = array('H', reference), array('H', raw(high['path']))
            error = sum(abs(x - y) for x, y in zip(before, after)) / len(before)
            assert len(before) == len(after) and error <= 4, error
            checks[-1]['highQualityMeanError10Bit'] = error
            edit['outputId'], edit['color'], edit['quality'] = 'pq-sdr', 'sdr', 'high'
            mutate('assemble', 'assemble-sdr', edit=edit)
            sdr = call('render', sequence='pq-sdr')
            sv = next(s for s in probe(sdr['path'])['streams'] if s['codec_type'] == 'video')
            assert sv['codec_name'] == 'h264' and sv['color_transfer'] == 'bt709'

    large_check = None
    # SDR preservation is also a distinct cut path. A lossless x264 master
    # needs a regular H.264 review encode for broad browser compatibility.
    sdr_source = root / 'sdr-source.mp4'
    run([a.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=640x360:r=30:d=2',
         '-c:v', 'libx264', '-threads', '2', '-crf', '0', '-pix_fmt', 'yuv420p',
         '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
         '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709',
         '-color_range', 'tv', sdr_source])
    mutate('import', 'import-sdr-native', source=str(sdr_source), id='sdr_source')
    mutate('assemble', 'assemble-sdr-native', edit={
        'outputId': 'sdr-native', 'name': 'SDR lossless source cuts', 'quality': 'lossless',
        'cuts': [{'id': 'take', 'assetId': 'sdr_source', 'startMs': 500, 'endMs': 1500}]})
    sdr_native = call('render', sequence='sdr-native')
    assert raw(sdr_native['path']) == raw(sdr_source, 0.5, 30)
    preview_artifact = call('artifact', id=sdr_native['artifactId'], preview=True)
    assert preview_artifact['role'] == 'sdr-review-preview' and preview_artifact['verified']
    assert Path(preview_artifact['path']).name == 'review-sdr.mp4'
    # Verify the selected preview, including rejection of corrupted bytes,
    # without making the valid master unavailable.
    preview_path = Path(preview_artifact['path'])
    preview_bytes = preview_path.read_bytes()
    preview_path.write_bytes(b'corrupt review preview')
    call('artifact', expect_ok=False, id=sdr_native['artifactId'], preview=True)
    assert call('artifact', id=sdr_native['artifactId'])['verified']
    preview_path.write_bytes(preview_bytes)
    if a.large_source:
        source = Path(a.large_source).resolve()
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        mutate('import', 'import-large-hlg', source=str(source), id='large_hlg')
        cuts = [(16900, 22800), (6100, 7500), (13800, 16000), (26300, 29900)]
        mutate('assemble', 'assemble-large-hlg', edit={
            'outputId': 'large-native', 'name': 'Full HD original cuts', 'allowReorder': True,
            'frameRate': {'numerator': 30, 'denominator': 1},
            'cuts': [{'id': 'c' + str(i), 'assetId': 'large_hlg', 'startMs': start, 'endMs': end}
                     for i, (start, end) in enumerate(cuts)]})
        result = call('render', sequence='large-native')
        manifest = json.loads(Path(result['manifest']).read_text())
        assert manifest['verification']['expectedFrames'] == 393
        assert manifest['outputColor']['transfer'] == 'arib-std-b67'
        assert manifest['verification']['decoded']
        assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
        assert manifest['resourcePolicy']['childAddressSpaceBytes'] == 4294967296
        large_check = {'frames': 393, 'size': [1080, 1920], 'transfer': 'arib-std-b67',
                       'sourceUnchanged': True, 'lossyIntermediate': False,
                       'workerAddressSpaceBytes': 4294967296}

    before = call('status')['revision']
    # Refusals happen before any timeline state is committed.
    for name, cuts in (
        ('mixed', [{'id': 'a', 'assetId': 'pq', 'startMs': 0, 'endMs': 1000},
                   {'id': 'b', 'assetId': 'hlg', 'startMs': 0, 'endMs': 1000}]),
        ('tight', [{'id': 'a', 'assetId': 'pq', 'startMs': 0, 'endMs': 200}]),
        ('reorder', [{'id': 'a', 'assetId': 'pq', 'startMs': 2000, 'endMs': 2500},
                     {'id': 'b', 'assetId': 'pq', 'startMs': 0, 'endMs': 1000}])):
        call('assemble', expect_ok=False, expectedRevision=before, key='reject-' + name,
             edit={'outputId': name, 'name': name, 'cuts': cuts})
        assert call('status')['revision'] == before
    summary = {'passed': True, 'scope': 'generated native 10-bit cut-only PQ/HLG; no display qualification',
               'checks': checks, 'fullHdRepeatedCuts': large_check,
               'losslessSdrPixelEquality': True, 'previewTamperRejected': True,
               'atomicRefusals': ['mixed transfers', 'tight cuts', 'unrequested reorder']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
