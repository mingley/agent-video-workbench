#!/usr/bin/env python3
"""Qualify a short 4K/60fps source to full-HD delivery; measure render time."""
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
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    ledger = []
    def run(argv):
        p = subprocess.run(list(map(str, argv)), capture_output=True, timeout=180)
        assert p.returncode == 0, (argv, p.stdout, p.stderr)
        return p.stdout
    def avw(*argv):
        response = json.loads(run([args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe, *argv]))
        ledger.append({'args':list(map(str, argv)), 'response':response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'], response
        return response['result']
    source = root / '4k60.mp4'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=3840x2160:r=60:d=2',
         '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=2',
         '-c:v', 'libx264', '-preset', 'ultrafast', '-threads', '2', '-pix_fmt', 'yuv420p',
         '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709', '-c:a', 'aac', source])
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    project = root / 'project'
    avw('create', project, '--name', '4K source')
    avw('import', project, source, '--id', 'source', '--expected-revision', 0, '--key', 'highres-source')
    avw('import', project, args.font, '--id', 'font', '--expected-revision', 1, '--key', 'highres-font')
    request = root / 'compose.json'
    request.write_text(json.dumps({'outputId':'hd', 'name':'Full HD from 4K/60', 'fontAssetId':'font',
        'cuts':[{'id':'cut', 'assetId':'source', 'startMs':500, 'endMs':1500}]}))
    avw('compose', project, '--request', request, '--expected-revision', 2, '--key', 'highres-compose')
    start = time.monotonic()
    artifact = avw('render', project, '--sequence', 'hd')
    elapsed = time.monotonic() - start
    manifest = json.loads(Path(artifact['manifest']).read_text())
    assert manifest['verification']['expectedFrames'] == 30, manifest
    assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
    summary = {'passed':True, 'sourceSize':[3840,2160], 'sourceFps':60, 'sourceSeconds':2,
        'outputSize':[1080,1920], 'outputFrames':30, 'renderWallSeconds':round(elapsed,3),
        'sourceUnchanged':True, 'limitations':['Short generated SDR fixture; no one-hour 4K performance or camera appearance claim']}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2))
    print('4K/60 fixture passed:', root / 'summary.json')


if __name__ == '__main__':
    main()
