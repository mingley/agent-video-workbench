#!/usr/bin/env python3
"""Interleave render verification timings with duplicate managed-original IDs.

Both binaries receive the same generated MP4. A valid sparse MP4 free box adds
64 MiB to the file, so hashing repeated asset records is measurable without
requiring private footage or network access. Source generation/import and
independent media comparisons are outside the measured verification stage.
"""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import statistics
import struct
import subprocess
import time


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('baseline', 'candidate', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--aliases', type=int, default=5,
                        help='Number of asset IDs bound to the same original')
    parser.add_argument('--padding-mib', type=int, default=64,
                        help='Size of a valid free box in MiB')
    args = parser.parse_args()
    if not 2 <= args.repeats <= 10:
        parser.error('--repeats must be in 2..10')
    if not 2 <= args.aliases <= 10:
        parser.error('--aliases must be in 2..10')
    if not 1 <= args.padding_mib <= 256:
        parser.error('--padding-mib must be in 1..256')

    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    binaries = {'baseline': str(Path(args.baseline).resolve()),
                'candidate': str(Path(args.candidate).resolve())}
    ledger = root / 'commands.jsonl'

    def run(argv, payload=None):
        command = list(map(str, argv))
        started = time.perf_counter()
        result = subprocess.run(command, input=payload, capture_output=True, timeout=600)
        elapsed = time.perf_counter() - started
        with ledger.open('a', encoding='utf-8') as log:
            log.write(json.dumps({'argv': command, 'exit': result.returncode,
                                  'seconds': elapsed}) + '\n')
        if result.returncode:
            raise AssertionError((command, result.stdout[-2000:], result.stderr[-2000:]))
        return result.stdout, elapsed

    def call(label, command, **fields):
        request = {'command': command, 'project': str(root / label), **fields}
        output, elapsed = run([binaries[label], '--ffmpeg', args.ffmpeg,
                               '--ffprobe', args.ffprobe, 'request', '-'],
                              json.dumps(request).encode())
        response = json.loads(output)
        if not response.get('ok'):
            raise AssertionError((request, response))
        return response['result'], elapsed

    source = root / 'generated.mp4'
    run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
         'testsrc2=s=640x360:r=30:d=6', '-f', 'lavfi', '-i',
         'sine=frequency=440:sample_rate=48000:duration=6',
         '-c:v', 'libx264', '-preset', 'ultrafast', '-threads', '2',
         '-pix_fmt', 'yuv420p', '-x264-params',
         'colorprim=bt709:transfer=bt709:colormatrix=bt709',
         '-color_range', 'tv', '-c:a', 'aac', '-movflags', '+faststart',
         '-shortest', source])
    free_bytes = args.padding_mib * 1024 * 1024
    with source.open('r+b') as video:
        video.seek(0, 2)
        offset = video.tell()
        video.write(struct.pack('>I4s', free_bytes, b'free'))
        video.seek(offset + free_bytes - 1)
        video.write(b'\0')
    probe, _ = run([args.ffprobe, '-v', 'error', '-show_entries',
                    'format=duration:stream=codec_type', '-of', 'json', source])
    probe = json.loads(probe)
    assert float(probe['format']['duration']) >= 5.9, probe
    assert {s['codec_type'] for s in probe['streams']} >= {'video', 'audio'}, probe
    source_hash = sha256(source)
    source_bytes = source.stat().st_size

    for label in binaries:
        call(label, 'create', name='Duplicate original benchmark')
        for index in range(args.aliases):
            call(label, 'import', source=str(source), id=f'source_{index}',
                 expectedRevision=index, key=f'fixture-source-{index:02d}')
        project, _ = call(label, 'status')
        assert len(project['assets']) == args.aliases, project['assets']
        assert {asset['uri'] for asset in project['assets']} == {f'originals/{source_hash}'}
        assert {asset['fingerprint']['sha256'] for asset in project['assets']} == {source_hash}
        call(label, 'assemble', edit={'outputId': 'short', 'name': 'Two-second cut',
                                     'cuts': [{'id': 'clip', 'assetId': 'source_0',
                                               'startMs': 1000, 'endMs': 3000}]},
             expectedRevision=args.aliases, key='fixture-assemble')

    samples = {'baseline': [], 'candidate': []}
    total_seconds = {'baseline': [], 'candidate': []}
    artifacts = []
    for iteration in range(args.repeats):
        order = ('baseline', 'candidate') if iteration % 2 == 0 else ('candidate', 'baseline')
        for label in order:
            artifact, elapsed = call(label, 'render', sequence='short')
            manifest = json.loads(Path(artifact['manifest']).read_text())
            assert manifest['verification']['expectedFrames'] == 60, manifest['verification']
            verification_seconds = manifest['timingsSeconds']['verifyOriginals']
            assert isinstance(verification_seconds, (float, int)) and verification_seconds >= 0
            samples[label].append(verification_seconds)
            total_seconds[label].append(elapsed)
            artifacts.append((label, iteration, Path(artifact['path'])))
            print(f'{label} run {iteration}: verifyOriginals={verification_seconds:.6f}s',
                  flush=True)

    def decoded_hash(path, kind):
        options = (['-map', '0:v:0', '-fps_mode', 'passthrough', '-pix_fmt', 'yuv420p']
                   if kind == 'video' else ['-map', '0:a:0', '-c:a', 'pcm_s16le'])
        out, _ = run([args.ffmpeg, '-v', 'error', '-xerror', '-threads', '1',
                      '-i', path, *options, '-f', 'hash', '-hash', 'sha256', '-'])
        return out.decode().strip()

    hashes = {'video': set(), 'audio': set()}
    for _, _, artifact in artifacts:
        for kind in hashes:
            hashes[kind].add(decoded_hash(artifact, kind))
    assert len(hashes['video']) == 1 and len(hashes['audio']) == 1, hashes
    assert sha256(source) == source_hash
    for label in binaries:
        managed = root / label / 'originals' / source_hash
        assert sha256(managed) == source_hash

    before = statistics.median(samples['baseline'])
    after = statistics.median(samples['candidate'])
    summary = {
        'passed': True,
        'aliasCount': args.aliases,
        'sourceBytes': source_bytes,
        'sourceSha256': source_hash,
        'freeBoxBytes': free_bytes,
        'verifyOriginals': {
            'beforeSeconds': samples['baseline'],
            'afterSeconds': samples['candidate'],
            'beforeMedianSeconds': before,
            'afterMedianSeconds': after,
            'medianReductionPercent': 100 * (1 - after / before) if before else None,
        },
        'endToEndSeconds': total_seconds,
        'decodedVideoEqual': True,
        'decodedAudioEqual': True,
        'originalUnchanged': True,
        'decodedVideoSha256': next(iter(hashes['video'])),
        'decodedAudioSha256': next(iter(hashes['audio'])),
        'binaries': {label: {'sha256': sha256(path),
                             'version': run([path, '--version'])[0].decode().strip()}
                     for label, path in binaries.items()},
        'platform': platform.platform(),
        'limits': [
            'Generated 640x360 SDR video with a sparse free box; no camera, HDR or 4K claim.',
            'Filesystem was warm; source generation, imports and independent decoding excluded.',
            'Identical content imported under several distinct IDs; unique-asset workloads may not improve.',
            'Timings are observational and have no pass/fail speed threshold.',
        ],
    }
    (root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('PERF_SUMMARY=' + json.dumps(summary, sort_keys=True, separators=(',', ':')),
          flush=True)


if __name__ == '__main__':
    main()
