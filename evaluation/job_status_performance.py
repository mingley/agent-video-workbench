#!/usr/bin/env python3
"""Compare warm CLI status latency on the same frozen jobs and project data."""
import argparse
import json
from pathlib import Path
import sqlite3
from statistics import median
import subprocess
from time import perf_counter


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('baseline', 'candidate', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--ffmpeg', default='ffmpeg')
    parser.add_argument('--ffprobe', default='ffprobe')
    args = parser.parse_args()
    binaries = [args.baseline.resolve(), args.candidate.resolve()]
    output = args.output.resolve()
    output.mkdir()
    project = output / 'project'

    def call(binary, *command, request=None):
        process = subprocess.run([binary, *command], capture_output=True, text=True,
                                 input=request, timeout=30)
        envelope = json.loads(process.stdout)
        assert process.returncode == 0 and envelope['ok'], envelope
        return envelope['result']

    def request(body):
        return call(binaries[0], 'request', '-', request=json.dumps(body))

    setup = call(binaries[0], 'create', project, '--name', 'Status benchmark')
    operations = [
        {'id': f'sequence-{i}', 'op': 'sequence.add', 'params': {
            'id': f'out_{i}', 'name': f'Output {i}', 'width': 360, 'height': 640,
            'frameRate': {'numerator': 30, 'denominator': 1},
        }}
        for i in range(1, 4)
    ]
    request({'command': 'apply', 'project': str(project), 'batch': {
        'schemaVersion': '1.0.0', 'projectId': setup['projectId'], 'baseRevision': 0,
        'idempotencyKey': 'status-bench-seed', 'operations': operations,
    }})
    batch = call(binaries[0], 'batch-start', project, '--sequences',
                 'seq_main,out_1,out_2,out_3', '--expected-revision', '1',
                 '--key', 'status-bench-batch', '--no-launch')
    checks = {
        'job-status': batch['result']['jobIds'][0],
        'batch-status': batch['id'],
    }

    def measure():
        with sqlite3.connect(project / 'project.sqlite') as connection:
            snapshot_bytes = connection.execute(
                'SELECT length(CAST(snapshot AS BLOB)) FROM revisions JOIN head ON id=revision'
            ).fetchone()[0]
        results = {'snapshotBytes': snapshot_bytes}
        for command, job_id in checks.items():
            expected = call(binaries[0], command, project, job_id)
            assert call(binaries[1], command, project, job_id) == expected
            samples = [[], []]
            for trial in range(30):
                for index in (trial % 2, 1 - trial % 2):
                    start = perf_counter()
                    actual = call(binaries[index], command, project, job_id)
                    samples[index].append((perf_counter() - start) * 1000)
                    assert actual == expected
            results[command] = {
                role: {'medianMs': round(median(values), 3),
                       'samplesMs': [round(value, 3) for value in values]}
                for role, values in zip(('baseline', 'candidate'), samples)
            }
        return results

    small = measure()
    source = output / 'silent.flac'
    subprocess.run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
                    'anullsrc=r=8000:cl=mono', '-t', '3600', '-c:a', 'flac', source],
                   check=True, timeout=30)
    call(binaries[0], '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
         'import', project, source, '--id', 'source', '--expected-revision', '1',
         '--key', 'status-bench-import')
    request({'command': 'transcript-import', 'project': str(project),
             'expectedRevision': 2, 'key': 'status-bench-transcript', 'transcript': {
                 'assetId': 'source', 'language': 'en', 'provider': 'benchmark',
                 'cues': [{'id': f'cue_{i}', 'startMs': i * 200, 'endMs': (i + 1) * 200,
                           'text': f'Synthetic transcript cue {i}. ' * 4}
                          for i in range(18_000)],
             }})
    evidence = {'passed': True, 'small': small, 'withTranscript': measure()}
    (output / 'summary.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(output / 'summary.json')


if __name__ == '__main__':
    main()
