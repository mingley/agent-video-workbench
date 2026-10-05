#!/usr/bin/env python3
"""Measure A/V sync and SDR compositing for a rotated, offset HDR/VFR source."""
import argparse
from array import array
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
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        assert result.returncode == 0, (argv, result.stdout, result.stderr)
        return result.stdout

    def call(command, expected=True, **fields):
        request = {'command': command, **fields}
        result = subprocess.run(
            [args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe, 'request', '-'],
            input=json.dumps(request).encode(), capture_output=True, timeout=180)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'exitCode': result.returncode, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == expected and (result.returncode == 0) == expected, response
        return response.get('result', response)

    def state():
        return call('status', project=str(project))

    # Matching light/tone bursts establish synchronization independently of the
    # application's graph. The audio starts 250ms late; the container starts at 2s.
    # Before 2s only every third video frame is retained. Both bursts begin on a
    # retained frame, allowing a one-output-frame alignment requirement.
    encoded = root / 'pq-encoded.mp4'
    source = root / 'phone-like-hdr.mov'
    hdr = ('setparams=range=limited:color_primaries=bt709:color_trc=bt709:colorspace=bt709,'
           'zscale=t=linear:npl=100,'
           'format=gbrpf32le,zscale=p=bt2020:t=smpte2084:m=bt2020nc:r=limited,'
           'format=yuv420p10le')
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'color=c=0x162040:s=640x360:r=30:d=4', '-f', 'lavfi', '-i',
         "aevalsrc='0.15*sin(2*PI*440*t)*(between(t,1,1.2)+between(t,2.5,2.7))':s=48000:d=4",
         '-vf', "drawbox=color=white:t=fill:enable='between(t,1,1.19)+between(t,2.5,2.69)',"
         "select='if(lt(t,2),not(mod(n,3)),1)'," + hdr,
         '-af', 'atrim=start=0.25,asetpts=PTS-STARTPTS+0.25/TB', '-fps_mode', 'vfr',
         '-c:v', 'libx265', '-tag:v', 'hvc1', '-x265-params',
         'pools=2:frame-threads=2:log-level=error:colorprim=bt2020:transfer=smpte2084:'
         'colormatrix=bt2020nc:master-display=G(13250,34500)B(7500,3000)R(34000,16000)'
         'WP(15635,16450)L(10000000,1):max-cll=1000,400',
         '-c:a', 'aac', '-output_ts_offset', '2', encoded])
    run([args.ffmpeg, '-v', 'error', '-display_rotation', '90', '-i', encoded,
         '-map', '0', '-c', 'copy', '-copyts', source])
    probe = json.loads(run([args.ffprobe, '-v', 'error', '-show_streams',
                           '-show_format', '-show_frames', '-of', 'json', source]))
    video = next(stream for stream in probe['streams'] if stream['codec_type'] == 'video')
    audio = next(stream for stream in probe['streams'] if stream['codec_type'] == 'audio')
    assert video['color_transfer'] == 'smpte2084' and video['pix_fmt'] == 'yuv420p10le'
    assert float(probe['format']['start_time']) >= 1.9, probe['format']
    assert float(audio['start_time']) - float(video['start_time']) > 0.2
    assert any(item.get('rotation') == 90 for item in video['side_data_list'])
    pts = [float(frame['best_effort_timestamp_time']) for frame in probe['frames']
           if frame['media_type'] == 'video']
    intervals = {round(b - a, 3) for a, b in zip(pts, pts[1:])}
    assert len(intervals) >= 2, intervals
    assert any(item['side_data_type'] == 'Mastering display metadata'
               for frame in probe['frames'] if frame['media_type'] == 'video'
               for item in frame.get('side_data_list', [])), 'HDR10 metadata missing'

    sources = {'pq': source}
    for name, transfer, color in [('hlg', 'arib-std-b67', 'green'), ('sdr', 'bt709', 'blue')]:
        path = root / (name + '.mp4')
        filters = ('setparams=range=limited:color_primaries=bt709:color_trc=bt709:colorspace=bt709,'
                   'zscale=t=linear:npl=100,'
                   f'format=gbrpf32le,zscale=p=bt2020:t={transfer}:m=bt2020nc:r=limited,'
                   'format=yuv420p10le') if name == 'hlg' else (
                       'setparams=range=limited:color_primaries=bt709:color_trc=bt709:colorspace=bt709,'
                       'format=yuv420p')
        run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
             f'color=c={color}:s=360x640:r=30:d=1', '-vf', filters,
             '-c:v', 'libx265' if name == 'hlg' else 'libx264',
             *(['-x265-params', 'pools=2:frame-threads=2:log-level=error:colorprim=bt2020:'
                'transfer=arib-std-b67:colormatrix=bt2020nc'] if name == 'hlg' else
               ['-preset', 'ultrafast', '-color_primaries', 'bt709', '-color_trc', 'bt709',
                '-colorspace', 'bt709']), path])
        sources[name] = path
    hashes = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in sources.items()}
    call('create', project=str(project), name='Combined HDR/VFR sync')
    for name, path in [*sources.items(), ('font', Path(args.font).resolve())]:
        call('import', project=str(project), source=str(path), id=name,
             expectedRevision=state()['revision'], key='combined-import-' + name)
    for asset, duration in [('pq', 4000), ('hlg', 1000), ('sdr', 1000)]:
        call('transcript-import', project=str(project), expectedRevision=state()['revision'],
             key='combined-transcript-' + asset, transcript={'assetId': asset, 'language': 'en',
             'provider': 'reviewed synthetic fixture', 'cues': [{'id': 'label', 'startMs': 0,
             'endMs': duration, 'text': 'WHITE'}]})
    call('compose', project=str(project), expectedRevision=state()['revision'],
         key='combined-compose', edit={'outputId': 'mixed', 'name': 'Mixed color and clocks',
         'fontAssetId': 'font', 'fontSize': 24, 'width': 360, 'height': 640,
         'cuts': [{'id': 'phone', 'assetId': 'pq', 'startMs': 500, 'endMs': 3500},
                  {'id': 'hlg', 'assetId': 'hlg', 'startMs': 0, 'endMs': 1000},
                  {'id': 'sdr', 'assetId': 'sdr', 'startMs': 0, 'endMs': 1000}]})
    artifact = call('render', project=str(project), sequence='mixed')
    manifest = json.loads(Path(artifact['manifest']).read_text())
    assert manifest['verification']['expectedFrames'] == 150, manifest['verification']
    output_probe = json.loads(run([args.ffprobe, '-v', 'error', '-show_streams',
                                  '-of', 'json', artifact['path']]))
    output_video = next(s for s in output_probe['streams'] if s['codec_type'] == 'video')
    assert all(output_video[key] == 'bt709' for key in
               ('color_primaries', 'color_transfer', 'color_space'))
    assert output_video['color_range'] == 'tv' and output_video['pix_fmt'] == 'yuv420p'
    assert (output_video['width'], output_video['height']) == (360, 640)
    decisions = {entry['assetId']: entry for entry in manifest['colorDecisions']}
    assert decisions['pq']['color']['hdr'] and decisions['hlg']['color']['hdr']
    assert decisions['sdr']['color']['hdr'] is False
    assert 'transpose' in decisions['pq']['sourceFilters']

    # Decode the entire delivery, measuring flash onsets and audio burst onsets.
    # Caption pixels are below the sampled center region.
    rgb = run([args.ffmpeg, '-v', 'error', '-i', artifact['path'], '-an',
               '-vf', 'scale=36:64', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])
    frame_bytes = 36 * 64 * 3
    assert len(rgb) == 150 * frame_bytes
    brightness = []
    for offset in range(0, len(rgb), frame_bytes):
        frame = rgb[offset:offset + frame_bytes]
        center = frame[(32 * 36 + 18) * 3:(32 * 36 + 18) * 3 + 3]
        brightness.append(sum(center) / 3)
    flash_onsets = [i / 30 for i, value in enumerate(brightness[:90])
                    if value > 150 and (i == 0 or brightness[i - 1] <= 150)]
    samples = array('f', run([args.ffmpeg, '-v', 'error', '-i', artifact['path'],
                            '-vn', '-ac', '1', '-ar', '48000', '-f', 'f32le', '-']))
    rms = [(sum(value * value for value in samples[i:i + 480]) / 480) ** 0.5
           for i in range(0, 3 * 48000, 480)]
    tone_onsets = [i / 100 for i, value in enumerate(rms)
                   if value > 0.025 and (i == 0 or rms[i - 1] <= 0.025)]
    assert len(flash_onsets) == len(tone_onsets) == 2, (flash_onsets, tone_onsets)
    sync_errors = [abs(light - tone) for light, tone in zip(flash_onsets, tone_onsets)]
    assert max(sync_errors) <= 1 / 30 + 0.01, (flash_onsets, tone_onsets, sync_errors)
    assert all(abs(actual - expected) <= 1 / 30 + 0.01
               for actual, expected in zip(flash_onsets, [0.5, 2.0])), flash_onsets
    assert max(rms[:35]) < 0.001, 'late audio gap was replaced by shifted sound'

    # Burned white captions belong to the SDR composite, including when the
    # underlying source is HDR. A matching white level detects double tone maps.
    def caption_peak(at):
        pixels = run([args.ffmpeg, '-v', 'error', '-ss', str(at), '-i', artifact['path'],
                      '-frames:v', '1', '-vf', 'crop=300:100:30:520', '-f', 'rawvideo',
                      '-pix_fmt', 'rgb24', '-'])
        neutral = [min(pixels[i:i + 3]) for i in range(0, len(pixels), 3)
                   if max(pixels[i:i + 3]) - min(pixels[i:i + 3]) < 10]
        assert neutral, 'no neutral caption pixels'
        return max(neutral)
    caption_peaks = [caption_peak(at) for at in (0.2, 3.2, 4.2)]
    assert min(caption_peaks) >= 240 and max(caption_peaks) - min(caption_peaks) <= 10, caption_peaks
    for name, path in sources.items():
        assert hashlib.sha256(path.read_bytes()).hexdigest() == hashes[name]
    call('delivery', project=str(project), id=artifact['artifactId'],
         destination=str(root / 'review'))
    verified = call('artifact', project=str(project), id=artifact['artifactId'])
    assert verified['verified'] is True
    native_edit = {'outputId': 'native_pq', 'name': 'Source-matched PQ with explicit CFR',
                   'frameRate': {'numerator': 30, 'denominator': 1}, 'quality': 'lossless',
                   'cuts': [{'id': 'phone', 'assetId': 'pq', 'startMs': 500, 'endMs': 3500}]}
    # Automatic frame-rate choice must refuse VFR instead of silently changing
    # timing. Explicit conformance retains source audio and records the finding.
    auto = dict(native_edit)
    del auto['frameRate']
    refused = call('assemble', expected=False, project=str(project),
                   expectedRevision=state()['revision'], key='native-vfr-needs-intent', edit=auto)
    assert 'explicit frameRate' in refused['error']['message'], refused
    call('assemble', project=str(project), expectedRevision=state()['revision'],
         key='native-vfr-explicit-rate', edit=native_edit)
    preflight = call('edit-preflight', project=str(project), sequence='native_pq')
    assert preflight['frameRate']['numerator'] == 30
    assert any('CFR' in f['finding'] for f in preflight['findings'])
    native = call('render', project=str(project), sequence='native_pq')
    native_manifest = json.loads(Path(native['manifest']).read_text())
    assert native_manifest['verification']['expectedFrames'] == 90
    assert native_manifest['outputColor']['transfer'] == 'smpte2084'
    nv = next(s for s in native_manifest['verification']['probe']['streams'] if s['codec_type'] == 'video')
    assert (nv['width'], nv['height']) == (360, 640) and nv['pix_fmt'] == 'yuv420p10le'
    tone_map = ('zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,'
                'tonemap=tonemap=mobius:param=0.3:desat=2:peak=10,'
                'zscale=t=bt709:m=bt709:r=limited:dither=error_diffusion,format=yuv420p')
    native_rgb = run([args.ffmpeg, '-v', 'error', '-threads', '1', '-i', native['path'],
                      '-an', '-vf', tone_map + ',scale=36:64', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])
    native_brightness = [sum(native_rgb[i + (32 * 36 + 18) * 3:i + (32 * 36 + 18) * 3 + 3]) / 3
                         for i in range(0, len(native_rgb), frame_bytes)]
    native_flashes = [i / 30 for i, v in enumerate(native_brightness)
                      if v > 150 and (i == 0 or native_brightness[i - 1] <= 150)]
    native_audio = array('f', run([args.ffmpeg, '-v', 'error', '-threads', '1', '-i', native['path'],
                                 '-vn', '-ac', '1', '-ar', '48000', '-f', 'f32le', '-']))
    native_rms = [(sum(v * v for v in native_audio[i:i + 480]) / 480) ** 0.5
                  for i in range(0, 3 * 48000, 480)]
    native_tones = [i / 100 for i, v in enumerate(native_rms)
                    if v > 0.025 and (i == 0 or native_rms[i - 1] <= 0.025)]
    assert len(native_flashes) == len(native_tones) == 2, (native_flashes, native_tones)
    native_sync = [abs(light - sound) for light, sound in zip(native_flashes, native_tones)]
    assert max(native_sync) <= 1 / 30 + 0.01
    assert all(abs(actual - expected) <= 1 / 30 + 0.01 for actual, expected
               in zip(native_flashes, [0.5, 2.0]))
    assert max(native_rms[:35]) < 0.001
    # Unsupported requests must refuse rather than silently produce an SDR
    # approximation or ignore authored effects. Earlier verified delivery stays.
    published = set(project.glob('renders/**/video.mp4'))
    revision = state()['revision']
    hdr_output = call('render-start', expected=False, project=str(project), sequence='mixed',
                      expectedRevision=revision, key='unsupported-hdr-output', output='HDR10')
    assert 'unknown field' in hdr_output['error']['message'], hdr_output
    assert state()['revision'] == revision
    wide = root / 'unqualified-wide.mp4'
    run([args.ffmpeg, '-v', 'error', '-i', sources['sdr'], '-vf',
         'zscale=p=bt2020:t=bt709:m=bt2020nc:r=limited', '-c:v', 'libx264',
         '-preset', 'ultrafast', '-color_primaries', 'bt2020', '-color_trc', 'bt709',
         '-colorspace', 'bt2020nc', wide])
    call('import', project=str(project), source=str(wide), id='wide',
         expectedRevision=revision, key='unqualified-wide-import')
    call('compose', project=str(project), expectedRevision=state()['revision'],
         key='unqualified-wide-compose', edit={'outputId': 'unsupported_wide', 'name': 'Unsupported wide color',
         'fontAssetId': 'font', 'fontSize': 24, 'width': 360, 'height': 640,
         'cuts': [{'id': 'wide', 'assetId': 'wide', 'startMs': 0, 'endMs': 1000}]})
    wide_error = call('render', expected=False, project=str(project), sequence='unsupported_wide')
    assert 'BT.2020 without PQ/HLG' in wide_error['error']['message'], wide_error
    current = state()
    call('apply', project=str(project), batch={'schemaVersion': '1.0.0',
         'projectId': current['projectId'], 'baseRevision': current['revision'],
         'idempotencyKey': 'unsupported-temperature-effect', 'operations': [{
             'id': 'temperature', 'op': 'effect.add', 'target': 'mixed_phone', 'params': {
                 'id': 'white_balance', 'effect': 'video.color.basic',
                 'params': {'temperature': 0.5}}}]})
    grade_error = call('render', expected=False, project=str(project), sequence='mixed')
    assert 'basic grade supports' in grade_error['error']['message'], grade_error
    assert set(project.glob('renders/**/video.mp4')) == published
    assert call('artifact', project=str(project), id=artifact['artifactId'])['sha256'] == verified['sha256']
    summary = {'passed': True, 'decodedFrames': 150, 'sourceSha256': hashes,
               'outputSha256': verified['sha256'], 'artifact': artifact['path'],
               'fixture': {'rotationDegrees': 90, 'pqBits': 10, 'vfrIntervalsSeconds': sorted(intervals),
                           'containerStartSeconds': float(probe['format']['start_time']),
                           'audioDelaySeconds': float(audio['start_time']) - float(video['start_time']),
                           'masteringDisplayMetadata': True},
               'flashOnsetsSeconds': flash_onsets, 'toneOnsetsSeconds': tone_onsets,
               'maxSyncErrorSeconds': max(sync_errors), 'captionWhitePeaksRgb': caption_peaks,
               'mixedPqHlgSdr': True, 'originalsUnchanged': True,
               'unsupportedPathsRejected': ['HDR-output request', 'unqualified BT.2020 transform',
                                            'basic white-balance parameter'],
               'priorVerifiedDeliveryRetained': True,
               'sourcePreservingPqVfr': {'decodedFrames': 90, 'explicitFrameRate': True,
                                         'flashOnsetsSeconds': native_flashes, 'toneOnsetsSeconds': native_tones,
                                         'maxSyncErrorSeconds': max(native_sync)},
               'limitations': ['Generated phone-like media; no real phone sensor, playback or appearance approval',
                               'HDR preservation is plain cuts only; proprietary Dolby Vision is not qualified']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print('Combined media passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
