#!/usr/bin/env python3
"""Exercise the real worker process: kill/recover, cancel, and retry a frozen job."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'project', 'output'):
        p.add_argument('--' + name, required=True)
    a = p.parse_args()
    output = Path(a.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    project = Path(a.project).resolve()
    ledger = []

    def command(*argv):
        run = subprocess.run([a.avw, '--ffmpeg', str(backend), '--ffprobe', a.ffprobe,
                              *map(str, argv)], capture_output=True, text=True, timeout=30)
        ledger.append({'args': list(map(str, argv)), 'exit': run.returncode,
                       'stdout': run.stdout, 'stderr': run.stderr})
        (output / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert run.returncode == 0, ledger[-1]
        response = json.loads(run.stdout)
        assert response['ok'], response
        return response['result']

    gate = output / 'gate'
    marker = output / 'child.pid'
    backend = output / 'controlled-ffmpeg'
    # Test-only executable shim. It execs, so the process-death signal reaches it.
    backend.write_text('#!/usr/bin/env python3\nimport os,sys,time\n'
        f'gate={str(gate)!r}\nmarker={str(marker)!r}\nactual={a.ffmpeg!r}\n'
        'if "-version" not in sys.argv and os.path.exists(gate):\n'
        ' open(marker,"w").write(str(os.getpid()))\n'
        ' os.execvp("sleep",["sleep","60"])\n'
        'os.execv(actual,[actual,*sys.argv[1:]])\n')
    backend.chmod(0o755)
    gate.touch()
    revision = command('status', project)['revision']
    job = command('render-start', project, '--expected-revision', revision,
                  '--key', 'kill-recovery', '--no-launch')
    assert command('render-start', project, '--expected-revision', revision,
                   '--key', 'kill-recovery', '--no-launch')['id'] == job['id']

    def worker():
        return subprocess.Popen([a.avw, 'worker', str(project)], stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True)

    def await_marker():
        for _ in range(100):
            if marker.exists():
                return int(marker.read_text())
            time.sleep(0.1)
        raise AssertionError('worker did not spawn the controlled backend')

    active = worker()
    child = await_marker()
    active.kill()
    active.communicate(timeout=5)
    for _ in range(50):
        stat = Path('/proc') / str(child) / 'stat'
        if not stat.exists() or stat.read_text().split()[2] == 'Z':
            break
        time.sleep(0.1)
    else:
        raise AssertionError('media child survived its killed execution owner')
    gate.unlink()
    marker.unlink()
    recovery = worker()
    recovery.communicate(timeout=10)
    assert recovery.returncode == 0
    assert command('job-status', project, job['id'])['state'] == 'interrupted'
    command('job-retry', project, job['id'], '--no-launch')
    retry = worker()
    retry.communicate(timeout=90)
    assert retry.returncode == 0
    completed = command('job-status', project, job['id'])
    assert completed['state'] == 'succeeded' and completed['attempt'] == 2, completed
    video = Path(completed['result']['path'])
    assert video.is_file()

    gate.touch()
    cancelled = command('render-start', project, '--expected-revision', revision,
                        '--key', 'cancel-test', '--no-launch')
    active = worker()
    await_marker()
    start = time.monotonic()
    command('job-cancel', project, cancelled['id'])
    active.communicate(timeout=5)
    assert active.returncode == 0
    assert command('job-status', project, cancelled['id'])['state'] == 'cancelled'
    assert time.monotonic() - start < 5
    assert video.is_file(), 'cancellation removed a previously verified output'
    (output / 'summary.json').write_text(json.dumps({'passed': True,
        'ownerDeathStopsChild': True, 'interruptedJobRecovered': True,
        'retryAttempt': 2, 'cancelBounded': True, 'previousFinalPreserved': True}, indent=2))
    print('Job smoke passed:', output / 'summary.json')


if __name__ == '__main__':
    main()
