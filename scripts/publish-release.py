#!/usr/bin/env python3
"""Publish only archives bound to a successful native qualification run."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile


def run(*args):
    return subprocess.check_output(list(map(str, args)), text=True).strip()


def main():
    output = Path(sys.argv[1])
    output.mkdir(parents=True, exist_ok=False)
    request = json.loads(Path('.github/release-request.json').read_text())
    tag, commit, run_id = request['tag'], request['commit'], request['qualificationRun']
    if not re.fullmatch(r'v\d+\.\d+\.\d+', tag) or not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('invalid release identity')
    qualification = json.loads(run('gh', 'run', 'view', run_id, '--json', 'headSha,status,conclusion,jobs'))
    if qualification['headSha'] != commit or qualification['status'] != 'completed' or qualification['conclusion'] != 'success':
        raise ValueError('release commit has not passed qualification')
    names = {job['name']: job['conclusion'] for job in qualification['jobs']}
    for name in ('qualify (ubuntu-24.04, x86_64)', 'qualify (ubuntu-24.04-arm, aarch64)'):
        if names.get(name) != 'success':
            raise ValueError('both native Linux qualification jobs must pass')
    assets = output / 'assets'
    assets.mkdir()
    checks, manifests = [], []
    for arch in ('x86_64', 'aarch64'):
        native = output / arch
        run('gh', 'run', 'download', run_id, '--name', f'qualified-native-{arch}-bundle', '--dir', native)
        required = {'application', 'jobs', 'matrix', 'agent', 'studio', 'audio',
                    'transfer', 'analysis', 'highres', 'timeline'}
        if tag != 'v0.3.0':
            required.add('combined')
            if arch == 'x86_64':
                required.add('browser')
        if tuple(map(int, tag[1:].split('.'))) >= (0, 4, 0):
            required.update({'color-memory', 'source-preserve', 'preserve-agent'})
            if arch == 'x86_64':
                required.add('preserve-browser')
        summaries = {p.parent.name: p for p in native.glob('*/summary.json')}
        if set(summaries) != required or any(
                json.loads(p.read_text()).get('passed') is not True for p in summaries.values()):
            raise ValueError('every required native workflow summary must pass')
        archive = native / 'release' / f'avw-{tag[1:]}-linux-{arch}.tar.gz'
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        expected = (native / 'release/SHA256SUMS').read_text().split()
        if expected != [actual, archive.name]:
            raise ValueError('native archive checksum mismatch')
        with tarfile.open(archive) as tar:
            manifest = json.load(tar.extractfile('avw/release.json'))
            if manifest['commit'] != commit or manifest['version'] != f'avw {tag[1:]}':
                raise ValueError('archive source/version does not match qualification')
            binary = tar.extractfile('avw/bin/avw').read()
            binary_sha = hashlib.sha256(binary).hexdigest()
            if tar.extractfile('avw/SHA256SUMS').read().decode().split() != [binary_sha, 'bin/avw']:
                raise ValueError('archive binary checksum mismatch')
            manifest.update(arch=arch, binarySha256=binary_sha, archiveSha256=actual)
            manifests.append(manifest)
            # Install helpers come from the qualified archive, not later branch edits.
            if arch == 'x86_64':
                for name in ('install-release.sh', 'setup-media.sh', 'setup-asr.sh'):
                    (assets / name).write_bytes(tar.extractfile(f'avw/scripts/{name}').read())
        shutil.copy(archive, assets / archive.name)
        checks.append(f'{actual}  {archive.name}')
    (assets / 'SHA256SUMS').write_text('\n'.join(checks) + '\n')
    (assets / 'native-manifests.json').write_text(json.dumps(manifests, indent=2) + '\n')
    notes = Path(request['notes'])
    if notes.parts[:2] != ('docs', 'releases') or not notes.is_file():
        raise ValueError('release notes must be a checked-in release document')
    result = subprocess.run(['gh', 'release', 'view', tag, '--json', 'isDraft,assets,targetCommitish'], capture_output=True, text=True)
    if result.returncode:
        run('gh', 'release', 'create', tag, '--draft', '--target', commit,
            '--title', f'Agent Video Workbench {tag[1:]}', '--notes-file', notes)
        release = json.loads(run('gh', 'release', 'view', tag, '--json', 'isDraft,assets,targetCommitish'))
    else:
        release = json.loads(result.stdout)
    if release['targetCommitish'] != commit:
        raise ValueError('existing release targets a different source; refusing replacement')
    # Retrying a partial upload verifies existing bytes rather than overwriting them.
    existing = {asset['name'] for asset in release['assets']}
    for file in sorted(assets.iterdir()):
        if file.name in existing:
            verified = output / ('existing-' + file.name)
            verified.mkdir()
            run('gh', 'release', 'download', tag, '--pattern', file.name, '--dir', verified)
            if hashlib.sha256((verified / file.name).read_bytes()).digest() != hashlib.sha256(file.read_bytes()).digest():
                raise ValueError('existing release asset differs; refusing replacement')
        elif release['isDraft']:
            run('gh', 'release', 'upload', tag, file)
        else:
            raise ValueError('published release is missing an expected asset')
    if release['isDraft']:
        run('gh', 'release', 'edit', tag, '--draft=false', '--notes-file', notes)
    print(run('gh', 'release', 'view', tag, '--json', 'url', '--jq', '.url'))


if __name__ == '__main__':
    main()
