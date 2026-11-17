"""Loopback-only TLS/HTTP protocol fixture, NOT a Matrix homeserver.

Generates disposable fixture keys under the caller-owned directory. No real
credentials, plaintext listener, personal trust-store changes or external I/O.
"""
import http.server
import json
import pathlib
import ssl
import subprocess
import sys

root = pathlib.Path(sys.argv[1])
mode = sys.argv[2]

def openssl(*args):
    result = subprocess.run(['openssl', *args], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if result.returncode:
        sys.stderr.buffer.write(result.stderr)
    result.check_returncode()

openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=Matrix Desktop test CA', '-keyout', str(root/'ca.key'), '-out', str(root/'ca.pem'))
openssl('req', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=localhost', '-keyout', str(root/'leaf.key'), '-out', str(root/'leaf.csr'))
(root/'ext.cnf').write_text('subjectAltName=DNS:localhost' + ('' if mode == 'wrong-host' else ',IP:127.0.0.1') + '\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n')
if mode == 'expired':
    # OpenSSL 3.0 x509 cannot set validity dates; ca supports explicit past dates.
    (root/'index.txt').touch()
    (root/'serial').write_text('01\n')
    (root/'ca.cnf').write_text(f'[ca]\ndefault_ca=fixture\n[fixture]\ndatabase={root}/index.txt\nserial={root}/serial\nnew_certs_dir={root}\ndefault_md=sha256\npolicy=policy\n[policy]\ncommonName=supplied\n')
    openssl('ca', '-batch', '-notext', '-config', str(root/'ca.cnf'), '-cert', str(root/'ca.pem'), '-keyfile', str(root/'ca.key'), '-in', str(root/'leaf.csr'), '-startdate', '20000101000000Z', '-enddate', '20000102000000Z', '-extfile', str(root/'ext.cnf'), '-out', str(root/'leaf.pem'))
else:
    openssl('x509', '-req', '-in', str(root/'leaf.csr'), '-CA', str(root/'ca.pem'), '-CAkey', str(root/'ca.key'), '-CAcreateserial', '-days', '1', '-extfile', str(root/'ext.cnf'), '-out', str(root/'leaf.pem'))

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def do_GET(self):
        with (root/'requests.jsonl').open('a') as out:
            out.write(json.dumps({'path': self.path, 'authorization': self.headers.get('Authorization') is not None})+'\n')
        status = 200
        headers = {}
        if self.path == '/.well-known/matrix/client':
            payload = {'m.homeserver': {'base_url': base+'/delegated'}}
            if mode == 'missing':
                status, payload = 404, {}
        elif self.path in ['/hop1', '/hop2', '/hop3']:
            payload = {'m.homeserver': {'base_url': base+'/delegated'}}
        elif self.path.endswith('/_matrix/client/versions'):
            payload = {'versions': ['v1.1', 'v1.15'], 'unstable_features': {'org.matrix.simplified_msc3575': True} if mode == 'sliding' else {}}
        elif self.path.endswith('/_matrix/client/v3/login'):
            payload = {'flows': [{'type': 'm.login.password'}, {'type': 'm.login.sso'}, {'type': 'm.login.token'}, {'type': 'org.example.future'}]}
        else:
            status, payload = 404, {}
        if mode == 'apex' and self.path.endswith(('/_matrix/client/versions', '/_matrix/client/v3/login')) and not self.path.startswith('/delegated'):
            status, payload = 404, {}
        if mode in ['redirect', 'loop']:
            chain = {'/.well-known/matrix/client': '/hop1', '/hop1': '/hop2', '/hop2': '/hop3'}
            if self.path in chain or (mode == 'loop' and self.path == '/hop3'):
                status = 302
                headers['Location'] = chain.get(self.path, '/hop1')
        if self.path == '/.well-known/matrix/client':
            if mode == 'bad-base':
                payload = {'m.homeserver': {'base_url': 'http://127.0.0.1/never'}}
            elif mode == 'bad-schema':
                payload = {'m.homeserver': {'base_url': 17}, 'private': 'fixture-sensitive-marker'}
            elif mode == 'server-error':
                status = 503
            elif mode.startswith('unsafe-redirect-'):
                status = 302
                headers['Location'] = {'http': 'http://127.0.0.1/never', 'userinfo': base.replace('https://', 'https://fixture:private@')+'/never', 'query': '/never?private', 'fragment': '/never#private', 'missing': '', 'ambiguous': 'https:127.0.0.1', 'empty-userinfo': base.replace('https://', 'https://@')+'/never'}[mode.removeprefix('unsafe-redirect-')]
                if not headers['Location']:
                    headers.pop('Location')
            elif mode == 'cross-origin':
                status = 307
                headers['Location'] = base.replace('127.0.0.1', 'localhost')+'/hop1'
        if self.path.endswith('/_matrix/client/versions'):
            if mode == 'bad-versions':
                payload = {'versions': []}
            elif mode == 'endpoint-redirect':
                status, headers['Location'] = 308, '/final-versions'
        if self.path == '/final-versions':
            status, payload = 200, {'versions': ['v1.15']}
        if self.path.endswith('/_matrix/client/v3/login'):
            if mode == 'bad-flows':
                payload = {'flows': [{'type': 17}]}
            elif mode == 'sso-only':
                payload = {'flows': [{'type': 'm.login.sso'}]}
            elif mode == 'empty-flows':
                payload = {'flows': []}
            elif mode == 'endpoint-redirect':
                status, headers['Location'] = 303, '/final-login'
        if self.path == '/final-login':
            status, payload = 200, {'flows': [{'type': 'm.login.password'}]}
        body = json.dumps(payload).encode()
        if self.path == '/_matrix/federation/v1/version':
            if mode == 'missing-server':
                status, body = 200, b'{}'
            elif mode == 'bad-json':
                status, body = 200, b'<html>not matrix</html>'
            elif mode == 'valid-federation':
                status, body = 200, b'{"server":{"name":"test","version":"1"}}'
        elif mode == 'bad-json':
            body = b'not-json-fixture-sensitive-marker'
        if mode in ['oversized', 'oversized-stream']:
            body = b' ' * 65537 + body
        self.send_response(status)
        for key, value in headers.items():
            self.send_header(key, value)
        self.send_header('Content-Type', 'application/json')
        if mode != 'oversized-stream':
            self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        if mode == 'slow':
            import time
            time.sleep(1)
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(root/'leaf.pem', root/'leaf.key')
server.socket = ctx.wrap_socket(server.socket, server_side=True)
base = f'https://127.0.0.1:{server.server_port}'
print(base, flush=True)
server.serve_forever()
