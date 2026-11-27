"""TLS loopback relay for a disposable server; hold only sliding-sync requests."""
import http.server
import json
import os
from pathlib import Path
import ssl
import sys
import time
import urllib.error
import urllib.request

config = json.loads(Path(os.environ["FIXTURE_AUTH_CONFIG"]).read_text())
root = Path(config["ca"]).parent
gate = root / "sliding-blocked"
observed = root / "sliding-observed"
failing = root / "sliding-failing"
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}),
    urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=config["ca"])),
)

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def forward(self):
        length = int(self.headers.get("Content-Length", "0"))
        if length > 1048576:
            self.send_error(413)
            return
        body = self.rfile.read(length) if length else None
        if self.command == "POST" and "org.matrix.simplified_msc3575/sync" in self.path:
            if failing.exists():
                self.send_error(503)
                return
            if gate.exists():
                observed.touch()
            while gate.exists():
                time.sleep(0.05)
        request = urllib.request.Request(
            config["url"].rstrip("/") + self.path, data=body, method=self.command,
            headers={k: self.headers[k] for k in ("Authorization", "Content-Type") if k in self.headers},
        )
        try:
            response = opener.open(request, timeout=45)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            payload = response.read()
            status = response.status
        assert isinstance(status, int)
        try:
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
            pass

    do_GET = forward
    do_POST = forward
    do_PUT = forward
    do_DELETE = forward

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.minimum_version = ssl.TLSVersion.TLSv1_2
context.load_cert_chain(root / "server.pem", root / "server.key")
server.socket = context.wrap_socket(server.socket, server_side=True)
print(f"https://localhost:{server.server_port}/", flush=True)
server.serve_forever()
