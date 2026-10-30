#!/usr/bin/env python3
"""Inspect bounded source ranges on generated nonzero-start A/V footage."""
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
    project = root / 'project'
    source = root / 'delayed-audio.mp4'
    ledger = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=120)
        assert result.returncode == 0, (argv, result.stderr.decode(errors='replace'))
        return result.stdout

    def call(command, expect_ok=True, **fields):
        request = {'command': command, 'project': str(project), **fields}
        result = subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                                 'request', '-'], input=json.dumps(request).encode(),
                                capture_output=True, timeout=120)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert (result.returncode == 0) == expect_ok and response['ok'] == expect_ok, response
        return response['result'] if expect_ok else response['error']

    # The container starts at 2 s, audio starts roughly 228 ms after video,
    # and synchronized flashes/tones occur at source times 1 s and 2.5 s.
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'color=c=black:s=320x180:r=30:d=4', '-f', 'lavfi', '-i',
         "aevalsrc='0.15*sin(2*PI*440*t)*(between(t,1,1.2)+between(t,2.5,2.7))':s=48000:d=4",
         '-f', 'lavfi', '-i', 'sine=frequency=880:sample_rate=48000:duration=4',
         '-vf', "drawbox=color=white:t=fill:enable='between(t,1,1.19)+between(t,2.5,2.69)'",
         '-filter_complex', '[1:a]atrim=start=0.25,asetpts=PTS-STARTPTS+0.25/TB[delayed]',
         '-map', '0:v:0', '-map', '[delayed]', '-map', '2:a:0',
         '-disposition:a:0', '0', '-disposition:a:1', 'default',
         '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
         '-c:a', 'aac', '-output_ts_offset', '2', source])
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    probe = json.loads(run([args.ffprobe, '-v', 'error', '-show_streams', '-show_format', '-of', 'json', source]))
    video = next(s for s in probe['streams'] if s['codec_type'] == 'video')
    audio = next(s for s in probe['streams'] if s['codec_type'] == 'audio')
    origin = float(probe['format']['start_time'])
    assert origin >= 1.9
    assert float(audio['start_time']) - float(video['start_time']) > 0.2
    flash_one = round((float(video['start_time']) - origin + 1.0) * 1000)
    flash_two = round((float(video['start_time']) - origin + 2.5) * 1000)
    call('create', name='Bounded inspection')
    call('import', source=str(source), id='source', expectedRevision=0, key='import-source')

    def inspect(kind, start, end):
        result = call('inspect', assetId='source', kind=kind, startMs=start, endMs=end)
        assert result['startMs'] == start and result['endMs'] == end
        assert result['timing']['mapping'] == 'reported-container-origin', result
        assert result['coverage']['leadingCoverage'] == 'not-measured', result
        assert result['coverage']['trailingCoverage'] == 'not-measured', result
        for item in result['evidence']:
            if item['sourceTimeMs'] is not None:
                assert start <= item['sourceTimeMs'] <= end, (start, end, item)
                if item['kind'] != 'silence_end':
                    assert item['sourceTimeMs'] < end, (start, end, item)
        return result

    for start, end, expected in [(0, flash_one - 50, []), (0, flash_one + 50, [flash_one]),
                                  (500, flash_one + 50, [flash_one]), (1500, flash_two - 50, []),
                                  (1500, flash_two + 50, [flash_two])]:
        result = inspect('scenes', start, end)
        assert result['analysisStatus'] == 'available', result
        assert [e['sourceTimeMs'] for e in result['evidence'] if e['kind'] == 'pts_time'] == expected, result

    # Before 550 ms there are fewer than 400 ms of decoded audio. The old
    # filter examined samples after the window and falsely reported silence.
    short = inspect('silence', 0, 550)
    assert short['analysisStatus'] == 'available', short
    assert not short['evidence'], short
    assert short['timing']['streamIndex'] == audio['index'], short
    assert 200 <= short['timing']['reportedStreamStartMs'] <= 260, short
    bounded = inspect('silence', 0, 800)
    assert [e['kind'] for e in bounded['evidence']] == ['silence_start', 'silence_end'], bounded
    assert 200 <= bounded['evidence'][0]['sourceTimeMs'] <= 260, bounded
    assert bounded['evidence'][1]['sourceTimeMs'] == 800, bounded
    assert bounded['evidence'][1]['boundaryMayBeTruncated'], bounded
    seek = inspect('silence', 500, 1500)
    assert seek['analysisStatus'] == 'available', seek
    assert seek['evidence'][0]['sourceTimeMs'] == 500, seek
    assert abs(seek['evidence'][1]['sourceTimeMs'] - flash_one) <= 2, seek
    beyond = inspect('silence', 0, 6000)
    assert beyond['coverage']['requestedEndExceedsReportedDuration'] is True, beyond
    assert beyond['analysisStatus'] == 'unavailable' and not beyond['evidence'], beyond
    cached = inspect('silence', 0, 800)
    assert cached['cacheHit'] and cached['evidence'] == bounded['evidence']
    assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
    assert call('status')['revision'] == 1

    # A discontinuous PCM timeline must not become a made-up silence interval.
    for name, discontinuous in [('continuous', False), ('gap', True)]:
        audio_source = root / f'{name}.mka'
        cmd = [args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'anullsrc=r=48000:cl=mono:d=2']
        if discontinuous:
            cmd += ['-af', "asetpts='if(gte(T,1),(T+3)/TB,PTS)'"]
        run([*cmd, '-c:a', 'pcm_s16le', audio_source])
        audio_project = root / f'{name}-project'
        call('create', project=str(audio_project), name=f'{name} audio')
        call('import', project=str(audio_project), source=str(audio_source), id='source',
             expectedRevision=0, key=f'import-{name}')
        end = 4500 if discontinuous else 1500
        result = call('inspect', project=str(audio_project), assetId='source', kind='silence', startMs=800, endMs=end)
        if discontinuous:
            assert result['analysisStatus'] == 'unavailable' and not result['evidence'], result
            assert result['coverage']['audioContinuity']['status'] == 'discontinuous', result
            assert not call('inspect', project=str(audio_project), assetId='source', kind='silence', startMs=800, endMs=end)['cacheHit']
        else:
            assert result['analysisStatus'] == 'available', result
            assert result['coverage']['audioContinuity']['status'] == 'contiguous-observed', result
            assert result['evidence'][0]['sourceTimeMs'] == 800, result
            assert result['evidence'][-1]['boundaryMayBeTruncated'], result

    # Coarse container timestamps and low sample rates must not create an
    # unpaired endpoint or move the observed sound boundary outside the range.
    for sample_rate in (48000, 100):
        pcm = root / f'pcm-{sample_rate}.mka'
        run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
             f"aevalsrc='if(between(t,1,1.2),0.2*sin(2*PI*10*t),0)':s={sample_rate}:d=2",
             '-c:a', 'pcm_s16le', pcm])
        pcm_project = root / f'pcm-{sample_rate}-project'
        call('create', project=str(pcm_project), name=f'PCM {sample_rate}')
        call('import', project=str(pcm_project), source=str(pcm), id='source', expectedRevision=0, key=f'import-pcm-{sample_rate}')
        for start, end in [(503, 1500), (0, 999), (0, 1001)]:
            result = call('inspect', project=str(pcm_project), assetId='source', kind='silence', startMs=start, endMs=end)
            assert result['analysisStatus'] == 'available', result
            assert [event['kind'] for event in result['evidence']] == ['silence_start', 'silence_end'], result
            assert result['evidence'][0]['sourceTimeMs'] == start, result
            assert result['evidence'][0]['boundaryMayBeTruncated'], result
            tolerance_ms = 3 + (1000 + sample_rate - 1) // sample_rate
            assert abs(result['evidence'][1]['sourceTimeMs'] - min(end, 1000)) <= tolerance_ms, result

    # Audio ending before the requested window is not evidence of sound
    # resumption: the decoded coverage is incomplete and findings are withheld.
    short_audio = root / 'short-audio.mp4'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=black:s=160x90:r=25:d=4',
         '-f', 'lavfi', '-i', 'anullsrc=r=48000:cl=mono:d=2', '-c:v', 'libx264', '-c:a', 'aac', short_audio])
    short_project = root / 'short-audio-project'
    call('create', project=str(short_project), name='Short audio')
    call('import', project=str(short_project), source=str(short_audio), id='source', expectedRevision=0, key='short-audio')
    incomplete = call('inspect', project=str(short_project), assetId='source', kind='silence', startMs=0, endMs=4000)
    assert incomplete['analysisStatus'] == 'unavailable' and not incomplete['evidence'], incomplete
    assert incomplete['coverage']['audioContinuity']['reason'] == 'audio-probe-trailing-coverage-incomplete', incomplete

    # TS stream origin differs from the container origin; source timestamps
    # must use the container. A long-GOP seek that loses its opening is explicit.
    gop_results = {}
    for name, short_gop in [('long', False), ('short', True)]:
        ts = root / f'{name}.ts'
        cmd = [args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=black:s=160x90:r=25:d=5',
               '-f', 'lavfi', '-i', "aevalsrc='0.15*sin(2*PI*440*t)*between(t,1,1.2)':s=48000:d=5",
               '-vf', "drawbox=color=white:t=fill:enable='between(t,1,1.19)'", '-c:v', 'libx264']
        if short_gop:
            cmd += ['-g', '12']
        else:
            cmd += ['-output_ts_offset', '0', '-avoid_negative_ts', 'disabled']
        run([*cmd, '-c:a', 'aac', ts])
        ts_probe = json.loads(run([args.ffprobe, '-v', 'error', '-show_streams', '-show_format', '-of', 'json', ts]))
        ts_video = next(s for s in ts_probe['streams'] if s['codec_type'] == 'video')
        expected = round((float(ts_video['start_time']) - float(ts_probe['format']['start_time']) + 1) * 1000)
        ts_project = root / f'{name}-ts-project'
        call('create', project=str(ts_project), name=f'{name} GOP')
        call('import', project=str(ts_project), source=str(ts), id='source', expectedRevision=0, key=f'import-{name}-ts')
        full = call('inspect', project=str(ts_project), assetId='source', kind='scenes', startMs=0, endMs=expected + 100)
        assert full['analysisStatus'] == 'available', full
        assert any(abs(e['sourceTimeMs'] - expected) <= 1 for e in full['evidence'] if e['kind'] == 'pts_time'), full
        sought = call('inspect', project=str(ts_project), assetId='source', kind='scenes', startMs=500, endMs=expected + 100)
        if sought['analysisStatus'] == 'available':
            assert any(abs(e['sourceTimeMs'] - expected) <= 1 for e in sought['evidence'] if e['kind'] == 'pts_time'), sought
        else:
            assert sought['unavailableReason'] in ('video-seek-leading-coverage-incomplete', 'no-decoded-video-frames', 'no-decoded-stream-samples'), sought
            assert not sought['evidence'], sought
        if short_gop:
            assert sought['analysisStatus'] == 'available', sought
        gop_results[name] = sought['analysisStatus']

    # Preserve millisecond boundaries when the source PTS clock is coarse.
    coarse = root / 'coarse.avi'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=black:s=160x90:r=25:d=1',
         '-vf', "drawbox=color=white:t=fill:enable='eq(n,2)'", '-c:v', 'ffv1', coarse])
    coarse_project = root / 'coarse-project'
    call('create', project=str(coarse_project), name='Coarse source clock')
    call('import', project=str(coarse_project), source=str(coarse), id='source', expectedRevision=0, key='import-coarse')
    for start, end, expected in [(21, 99, [80]), (21, 119, [80]), (0, 81, [80]), (0, 80, [])]:
        result = call('inspect', project=str(coarse_project), assetId='source', kind='scenes', startMs=start, endMs=end)
        assert result['analysisStatus'] == 'available', result
        assert [item['sourceTimeMs'] for item in result['evidence'] if item['kind'] == 'pts_time'] == expected, result
        if start == 21:
            assert result['coverage']['firstDecodedVideoTimeMs'] == 40, result

    # Attached artwork on an audio-only source is not a scene-video stream.
    artwork, music = root / 'cover.jpg', root / 'music.mp3'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=red:s=64x64',
         '-frames:v', '1', '-c:v', 'mjpeg', '-threads', '1', artwork])
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=2',
         '-i', artwork, '-map', '0:a:0', '-map', '1:v:0', '-c:a', 'libmp3lame', '-c:v', 'copy',
         '-disposition:v:0', 'attached_pic', music])
    music_probe = json.loads(run([args.ffprobe, '-v', 'error', '-show_streams', '-of', 'json', music]))
    assert any(s.get('disposition', {}).get('attached_pic') == 1 for s in music_probe['streams']), music_probe
    music_project = root / 'music-project'
    call('create', project=str(music_project), name='Audio with artwork')
    call('import', project=str(music_project), source=str(music), id='source', expectedRevision=0, key='import-music')
    art_error = call('inspect', expect_ok=False, project=str(music_project), assetId='source', kind='scenes', startMs=0, endMs=1000)
    assert 'attached artwork' in art_error['message'], art_error

    summary = {'passed': True, 'sourceUnchanged': True, 'containerStartSeconds': float(probe['format']['start_time']),
               'reportedAudioStartMs': short['timing']['reportedStreamStartMs'],
               'sceneWindowsBounded': True, 'shortWindowHasNoFalseSilence': True,
               'boundaryTruncationReported': True, 'nonzeroSeekVerified': True,
               'firstAudioStreamSelectedEvenWhenAnotherIsDefault': True,
               'unknownStreamEdgeCoverageReported': True, 'cacheReplay': True,
               'discontinuousAudioUnavailable': True, 'shortAudioUnavailable': True,
               'mpegTsSeek': gop_results, 'coarseTimebaseHalfOpenBounds': True,
               'pcmSampleBoundaryPairing': True, 'attachedArtworkRefused': True}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('Bounded inspection passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
