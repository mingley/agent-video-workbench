#!/usr/bin/env python3
"""Measure sidechain attenuation and preserve unprocessed history."""
import argparse
from array import array
import json
import math
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

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=90)
        assert result.returncode == 0, (argv, result.stdout, result.stderr)
        return result.stdout

    def call(command, **fields):
        request = {'command': command, **fields}
        response = json.loads(subprocess.run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                              'request', '-'], input=json.dumps(request).encode(), capture_output=True,
                              timeout=90).stdout)
        assert response['ok'], response
        return response['result']

    voice = root / 'voice.mp4'
    music = root / 'music.wav'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=blue:s=360x640:r=30:d=4',
         '-f', 'lavfi', '-i', 'sine=frequency=440:duration=4:sample_rate=48000', '-af',
         "volume='if(between(t,1,3),1,0)':eval=frame", '-c:v', 'libx264', '-c:a', 'aac', voice])
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'sine=frequency=880:duration=4:sample_rate=48000', music])
    call('create', project=str(project), name='Audio suite')
    for revision, (asset, path) in enumerate([('voice', voice), ('music', music)]):
        call('import', project=str(project), source=str(path), id=asset,
             expectedRevision=revision, key='import-' + asset)
    p = call('status', project=str(project))
    time = lambda value: {'value': value, 'rate': {'numerator': 30, 'denominator': 1}}
    call('apply', project=str(project), batch={'schemaVersion': '1.0.0', 'projectId': p['projectId'],
         'baseRevision': 2, 'idempotencyKey': 'add-audio', 'operations': [
             {'id': 'voice', 'op': 'clip.add', 'params': {'id': 'voice_clip', 'asset': 'voice',
              'track': 'track_v1', 'at': time(0), 'sourceIn': time(0), 'duration': time(120)}},
             {'id': 'music', 'op': 'clip.add', 'params': {'id': 'music_clip', 'asset': 'music',
              'track': 'track_a1', 'at': time(0), 'sourceIn': time(0), 'duration': time(120)}}]})
    before = call('render', project=str(project))
    call('studio', project=str(project), expectedRevision=3, key='duck-music', edit={
         'action': 'audio-duck', 'sequence': 'seq_main', 'settings': {'dialogueItems': ['voice_clip'],
         'musicItems': ['music_clip'], 'threshold': 0.01, 'ratio': 10, 'attackMs': 20, 'releaseMs': 250}})
    after = call('render', project=str(project))

    def amplitude(path, start, frequency):
        data = array('f')
        data.frombytes(run([args.ffmpeg, '-v', 'error', '-ss', str(start), '-i', path,
                            '-t', '0.5', '-vn', '-ac', '1', '-ar', '16000', '-f', 'f32le', '-']))
        real = sum(value * math.cos(2 * math.pi * frequency * i / 16000) for i, value in enumerate(data))
        imag = sum(value * math.sin(2 * math.pi * frequency * i / 16000) for i, value in enumerate(data))
        return 2 * math.hypot(real, imag) / len(data)

    speech_before = amplitude(before['path'], 1.5, 440)
    speech_after = amplitude(after['path'], 1.5, 440)
    music_before = amplitude(before['path'], 1.5, 880)
    music_after = amplitude(after['path'], 1.5, 880)
    attenuation = 20 * math.log10(music_after / music_before)
    assert attenuation < -6, attenuation
    assert abs(speech_after / speech_before - 1) < 0.05
    assert amplitude(after['path'], 0.1, 880) / amplitude(before['path'], 0.1, 880) > 0.9
    call('restore', project=str(project), revision=3, expectedRevision=4, key='undo-duck')
    undo = call('render', project=str(project))
    assert abs(amplitude(undo['path'], 1.5, 880) / music_before - 1) < 0.01
    p = call('status', project=str(project))
    call('apply', project=str(project), batch={'schemaVersion':'1.0.0','projectId':p['projectId'],
         'baseRevision':p['revision'],'idempotencyKey':'fade-music-effect','operations':[
         {'id':'fade','op':'effect.add','target':'music_clip','params':{'id':'fade','effect':'audio.fade',
          'params':{'inSeconds':1,'outSeconds':1}}}]})
    faded = call('render', project=str(project))
    assert amplitude(faded['path'], 0.1, 880)/amplitude(before['path'], 0.1, 880)<.5
    assert .9<amplitude(faded['path'], 1.5, 880)/music_before<1.1
    assert amplitude(faded['path'], 3.4, 880)/amplitude(before['path'], 3.4, 880)<.5
    summary = {'passed': True, 'musicAttenuationDb': attenuation,
               'speechAmplitudeRatio': speech_after / speech_before,
               'checks': ['real sidechain attenuation during speech', 'effect-based in/out fades', 'music retained in silence',
                          'dialogue amplitude/timing retained', 'history restores unprocessed mix']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print('Audio passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
