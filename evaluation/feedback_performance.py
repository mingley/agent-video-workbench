#!/usr/bin/env python3
"""Compare two AVW binaries on generated 60s input / 30s, 720p cut exports.

Run without competing builds/jobs. This is a CPU synthetic benchmark, not a
claim about camera footage, 4K, HDR, transcription, GPU or end-user hosts.
"""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import statistics
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
for arg in ('before', 'after', 'ffmpeg', 'ffprobe', 'output'):
    p.add_argument('--' + arg, required=True)
p.add_argument('--repeats', type=int, default=3)
a = p.parse_args()
assert a.repeats > 0
root = Path(a.output).resolve()
root.mkdir(parents=True, exist_ok=False)
ledger = []


def run(argv, data=None):
    start = time.perf_counter()
    result = subprocess.run(list(map(str, argv)), input=data, capture_output=True, timeout=600)
    elapsed = time.perf_counter() - start
    assert result.returncode == 0, (argv, result.stdout, result.stderr)
    return result.stdout, elapsed


def call(label, command, **fields):
    request = dict(command=command, project=str(root / label), **fields)
    output, elapsed = run([getattr(a, label), '--ffmpeg', a.ffmpeg, '--ffprobe', a.ffprobe,
                           'request', '-'], json.dumps(request).encode())
    response = json.loads(output)
    assert response['ok'], response
    ledger.append(dict(binary=label, request=request, response=response, elapsedSeconds=elapsed))
    (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
    return response['result'], elapsed


source = root / 'source.mp4'
run([a.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=1280x720:r=30:d=60',
     '-f', 'lavfi', '-i', 'sine=frequency=440:duration=60', '-c:v', 'libx264',
     '-preset', 'ultrafast', '-threads', '2', '-pix_fmt', 'yuv420p',
     '-x264-params', 'colorprim=bt709:transfer=bt709:colormatrix=bt709',
     '-color_range', 'tv', '-c:a', 'aac', '-shortest', source])
source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
results = {label: [] for label in ('before', 'after')}
paths = {}
for label in results:
    call(label, 'create', name='Feedback performance fixture')
    call(label, 'import', source=str(source), id='source', expectedRevision=0, key='import-source')
    call(label, 'assemble', edit={'outputId': 'long', 'name': '30 second output', 'cuts': [
        {'id': 'first', 'assetId': 'source', 'startMs': 15000, 'endMs': 30000},
        {'id': 'second', 'assetId': 'source', 'startMs': 40000, 'endMs': 55000}]},
        expectedRevision=1, key='assemble-long')
for iteration in range(a.repeats):
    # Alternate order to reduce systematic warm-cache/order bias.
    for label in (('before', 'after') if iteration % 2 == 0 else ('after', 'before')):
        artifact, elapsed = call(label, 'render', sequence='long')
        manifest = json.loads(Path(artifact['manifest']).read_text())
        assert manifest['verification']['expectedFrames'] == 900
        results[label].append(elapsed)
        paths[label] = artifact['path']
        print(label, iteration, round(elapsed, 3), flush=True)

# Independently decode delivered pixels AND audio for before/after equality.
hashes = {}
for label, path in paths.items():
    hashes[label] = {}
    for kind, args in [('video', ['-map', '0:v:0', '-fps_mode', 'passthrough', '-pix_fmt', 'yuv420p']),
                       ('audio', ['-map', '0:a:0', '-c:a', 'pcm_s16le'])]:
        raw, _ = run([a.ffmpeg, '-v', 'error', '-threads', '1', '-i', path, *args,
                      '-f', 'hash', '-hash', 'sha256', '-'])
        hashes[label][kind] = raw.decode().strip()
assert hashes['before'] == hashes['after'], hashes
assert hashlib.sha256(source.read_bytes()).hexdigest() == source_hash
# Isolate the two original verification subprocesses from the single-decode
# replacement on the same rendered file. Hashing/contract checks are unchanged
# and excluded here; end-to-end timings above include them.
verification = {label: [] for label in results}
for iteration in range(a.repeats):
    for label in (('before', 'after') if iteration % 2 == 0 else ('after', 'before')):
        decode = [a.ffmpeg, '-v', 'error', '-xerror', '-threads', '1', '-i', paths['after']]
        if label == 'after':
            decode += ['-map', '0:v:0', '-map', '0:a?', '-fps_mode', 'passthrough',
                       '-nostats', '-stats_period', '3600', '-progress', 'pipe:1']
        decoded, decode_seconds = run([*decode, '-f', 'null', '-'])
        probe = [a.ffprobe, '-v', 'error']
        if label == 'before':
            probe += ['-count_frames']
        metadata, probe_seconds = run([*probe, '-threads', '1', '-show_streams',
                                       '-show_format', '-of', 'json', paths['after']])
        if label == 'before':
            video = next(v for v in json.loads(metadata)['streams'] if v['codec_type'] == 'video')
            assert video['nb_read_frames'] == '900'
        else:
            progress = dict(line.split('=', 1) for line in decoded.decode().splitlines() if '=' in line)
            assert progress['progress'] == 'end' and int(progress['frame']) == 900
        verification[label].append(decode_seconds + probe_seconds)
summary = dict(passed=True, sourceSeconds=60, outputSeconds=30, outputFrames=900,
               canvas='1280x720', fps=30, repeatedSeconds=results,
               verificationSubprocessSeconds=verification,
               verificationMedianSeconds={k: statistics.median(v) for k, v in verification.items()},
               medianSeconds={k: statistics.median(v) for k, v in results.items()},
               decodedHashes=hashes, originalUnchanged=True,
               platform=platform.platform(),
               cpuModel=next((s.split(':', 1)[1].strip() for s in Path('/proc/cpuinfo').read_text().splitlines()
                              if s.startswith('model name')), 'unknown'),
               cpuQuota=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),
               memoryLimitBytes=Path('/sys/fs/cgroup/memory.max').read_text().strip(),
               binaries={k: hashlib.sha256(Path(getattr(a, k)).read_bytes()).hexdigest() for k in results},
               ffmpeg=run([a.ffmpeg, '-version'])[0].decode().splitlines()[0],
               limitations=['Synthetic SDR testsrc2 plus sine audio; no camera/HDR/4K/ASR claim',
                            'CPU only; shared virtual host, warm filesystem cache; elapsed times vary',
                            'No peak RSS measurement; existing 4GiB child limit retained'])
summary['medianReductionPercent'] = 100 * (1 - summary['medianSeconds']['after'] / summary['medianSeconds']['before'])
(root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
