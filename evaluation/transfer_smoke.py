#!/usr/bin/env python3
"""Interrupted/ranged transfer, changed identity and source-scope regression."""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import subprocess
import threading


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw', 'ffmpeg', 'ffprobe', 'output'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    source = root / 'source.mp4'
    subprocess.run([args.ffmpeg, '-v', 'error', '-f', 'lavfi', '-i',
                    'testsrc2=s=640x360:r=30:d=3', '-c:v', 'libx264', '-preset', 'ultrafast', source], check=True)
    payload = source.read_bytes()
    digest = hashlib.sha256(payload).hexdigest()
    state = {'etag': 'version1', 'interrupt': True, 'range': []}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_HEAD(self):
            if self.path.startswith('/redirect'):
                self.send_response(302)
                self.send_header('Location', 'http://127.0.0.2/private')
                self.end_headers()
                return
            self.send_response(200)
            self.send_header('Content-Length', str(len(payload)))
            self.send_header('ETag', '"' + state['etag'] + '"')
            self.end_headers()

        def do_GET(self):
            header = self.headers.get('Range')
            offset = int(header.split('=')[1].split('-')[0]) if header else 0
            state['range'].append(offset)
            self.send_response(206 if offset else 200)
            self.send_header('ETag', '"' + state['etag'] + '"')
            self.send_header('Content-Length', str(len(payload) - offset))
            if offset:
                self.send_header('Content-Range', f'bytes {offset}-{len(payload)-1}/{len(payload)}')
            self.end_headers()
            if state['interrupt']:
                state['interrupt'] = False
                self.wfile.write(payload[offset:offset + len(payload)//3])
                self.wfile.flush()
                self.connection.shutdown(socket.SHUT_RDWR)
                self.connection.close()
            else:
                self.wfile.write(payload[offset:])

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    endpoint = f'http://localhost:{server.server_port}'
    ledger = []

    def call(request, ok=True):
        command = [args.avw, '--ffmpeg', args.ffmpeg, '--ffprobe', args.ffprobe,
                   '--download-host', 'localhost', '--download-loopback-http', 'request', '-']
        result = subprocess.run(command, input=json.dumps(request).encode(), capture_output=True, timeout=30)
        response = json.loads(result.stdout)
        ledger.append({'request': request, 'response': response})
        (root / 'commands.json').write_text(json.dumps(ledger, indent=2))
        assert response['ok'] == ok, response
        return response.get('result', response)

    def request(project, path='/media?secret-fixture-token', sha=digest):
        return {'command': 'import-url', 'project': str(project), 'url': endpoint + path,
                'id': 'phone', 'expectedRevision': 0, 'key': 'download-phone', 'sha256': sha}

    try:
        project = root / 'resumed'
        call({'command': 'create', 'project': str(project), 'name': 'Resumed transfer'})
        call(request(project), ok=False)
        assert not list((project / 'originals').iterdir())
        assert list((project / 'cache').glob('download-*.part'))
        result = call(request(project))
        assert result['transfer']['resumedFromBytes'] > 0
        assert state['range'] == [0, len(payload)//3], state
        assert hashlib.sha256((project / 'originals' / digest).read_bytes()).hexdigest() == digest
        assert b'secret-fixture-token' not in (project / 'project.sqlite').read_bytes()
        changed = root / 'changed'
        call({'command': 'create', 'project': str(changed), 'name': 'Changed validator'})
        state['interrupt'] = True
        call(request(changed), ok=False)
        state['etag'] = 'version2'
        result = call(request(changed))
        assert result['transfer']['resumedFromBytes'] == 0
        wrong = root / 'wrong'
        call({'command': 'create', 'project': str(wrong), 'name': 'Wrong checksum'})
        call(request(wrong, sha='0' * 64), ok=False)
        assert not list((wrong / 'originals').iterdir())
        call(request(wrong, '/redirect'), ok=False)
        call({**request(wrong), 'url': 'https://not-authorized.example/media'}, ok=False)
        summary = {'passed': True, 'checks': ['partial transfer resumes with strong ETag',
                   'changed validator restarts', 'wrong checksum preserves head/originals',
                   'redirect destinations and initial hosts scoped', 'signed URL absent from SQLite']}
        (root / 'summary.json').write_text(json.dumps(summary, indent=2))
        print('Transfer passed:', root / 'summary.json')
    finally:
        server.shutdown()


if __name__ == '__main__':
    main()
